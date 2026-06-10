use serde::Serialize;
use std::time::Instant;

use super::DiagnosticResult;

#[derive(Debug, Clone, Serialize)]
pub struct DnsInfo {
    pub servers: Vec<DnsServer>,
    /// The `dns.google` probe, kept for JSON backward compatibility.
    pub resolution_test: Option<DnsResolutionTest>,
    /// All probed domains (additive field, v3.4.0+).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub resolution_tests: Vec<DnsResolutionTest>,
}

#[derive(Debug, Clone, Serialize)]
pub struct DnsServer {
    pub address: String,
    pub reachable: bool,
    pub latency_ms: Option<f64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct DnsResolutionTest {
    pub domain: String,
    pub resolved: bool,
    pub resolution_time_ms: f64,
    pub resolved_ips: Vec<String>,
}

pub async fn check() -> (DiagnosticResult, Option<DnsInfo>) {
    let servers = get_dns_servers().await;

    if servers.is_empty() {
        return (
            DiagnosticResult::fail("DNS", "No DNS servers configured"),
            None,
        );
    }

    // Test DNS resolution against several independent domains concurrently.
    // The verdict uses the count resolved + the median time, so one slow or
    // dropped lookup can no longer flip the verdict on a healthy network.
    let tests: Vec<DnsResolutionTest> =
        futures_util::future::join_all(TEST_DOMAINS.iter().map(|d| test_domain(d))).await;

    let server_results: Vec<DnsServer> = servers
        .iter()
        .map(|s| DnsServer {
            address: s.clone(),
            reachable: true, // We know they're configured
            latency_ms: None,
        })
        .collect();

    let info = DnsInfo {
        servers: server_results,
        resolution_test: tests.iter().find(|t| t.domain == TEST_DOMAINS[0]).cloned(),
        resolution_tests: tests.clone(),
    };

    let result = dns_verdict(&tests);

    (result, Some(info))
}

/// Pure verdict over the resolution probes — unit-testable without a network.
///
/// - 0 resolved → Fail
/// - some (but not all) resolved → Warn (partial resolution)
/// - all resolved → verdict on the **median** resolution time
///   (≤200ms Ok, 200–500ms Warn, >500ms Warn-slow)
fn dns_verdict(tests: &[DnsResolutionTest]) -> DiagnosticResult {
    let total = tests.len();
    let mut resolved_times: Vec<f64> = tests
        .iter()
        .filter(|t| t.resolved)
        .map(|t| t.resolution_time_ms)
        .collect();

    if resolved_times.is_empty() {
        return DiagnosticResult::fail("DNS", "DNS resolution failed");
    }

    if resolved_times.len() < total {
        return DiagnosticResult::warn(
            "DNS",
            format!(
                "Partial DNS resolution ({}/{} domains)",
                resolved_times.len(),
                total
            ),
        );
    }

    resolved_times.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let median = resolved_times[resolved_times.len() / 2];

    if median > 500.0 {
        DiagnosticResult::warn("DNS", format!("Resolving slowly ({:.0}ms median)", median))
    } else if median > 200.0 {
        DiagnosticResult::warn(
            "DNS",
            format!("Resolving with moderate latency ({:.0}ms median)", median),
        )
    } else {
        DiagnosticResult::ok(
            "DNS",
            format!("Resolving normally ({:.0}ms median)", median),
        )
    }
}

async fn get_dns_servers() -> Vec<String> {
    #[cfg(windows)]
    {
        get_dns_servers_windows().await
    }

    #[cfg(target_os = "macos")]
    {
        get_dns_servers_macos().await
    }

    #[cfg(target_os = "linux")]
    {
        get_dns_servers_linux().await
    }
}

#[cfg(windows)]
async fn get_dns_servers_windows() -> Vec<String> {
    let mut servers = Vec::new();

    let mut cmd = tokio::process::Command::new("netsh");
    cmd.args(["interface", "ip", "show", "dns"]);
    if let Some(output) = super::util::run_with_timeout(cmd, super::util::QUICK).await {
        let text = String::from_utf8_lossy(&output.stdout);
        for line in text.lines() {
            let line = line.trim();
            // Look for IP addresses in the output
            if let Some(ip) = extract_ip(line) {
                if !servers.contains(&ip) {
                    servers.push(ip);
                }
            }
        }
    }

    if servers.is_empty() {
        // Fallback: try ipconfig
        let mut cmd = tokio::process::Command::new("ipconfig");
        cmd.args(["/all"]);
        if let Some(output) = super::util::run_with_timeout(cmd, super::util::QUICK).await {
            let text = String::from_utf8_lossy(&output.stdout);
            let mut in_dns_section = false;
            for line in text.lines() {
                if line.contains("DNS Servers") {
                    in_dns_section = true;
                    if let Some(ip) = extract_ip(line) {
                        servers.push(ip);
                    }
                } else if in_dns_section {
                    let trimmed = line.trim();
                    if trimmed.is_empty()
                        || trimmed.contains(':') && !trimmed.starts_with(char::is_numeric)
                    {
                        in_dns_section = false;
                    } else if let Some(ip) = extract_ip(trimmed) {
                        servers.push(ip);
                    }
                }
            }
        }
    }

    servers
}

#[cfg(target_os = "macos")]
async fn get_dns_servers_macos() -> Vec<String> {
    let mut servers = Vec::new();

    let mut cmd = tokio::process::Command::new("scutil");
    cmd.args(["--dns"]);
    if let Some(output) = super::util::run_with_timeout(cmd, super::util::QUICK).await {
        let text = String::from_utf8_lossy(&output.stdout);
        for line in text.lines() {
            if line.contains("nameserver") {
                if let Some(ip) = extract_ip(line) {
                    if !servers.contains(&ip) {
                        servers.push(ip);
                    }
                }
            }
        }
    }

    servers
}

#[cfg(target_os = "linux")]
async fn get_dns_servers_linux() -> Vec<String> {
    let mut servers = Vec::new();

    // Try /etc/resolv.conf first
    if let Ok(content) = tokio::fs::read_to_string("/etc/resolv.conf").await {
        for line in content.lines() {
            if line.starts_with("nameserver") {
                if let Some(ip) = line.split_whitespace().nth(1) {
                    servers.push(ip.to_string());
                }
            }
        }
    }

    // Fallback: try resolvectl
    if servers.is_empty() || servers.iter().all(|s| s == "127.0.0.53") {
        let mut cmd = tokio::process::Command::new("resolvectl");
        cmd.args(["status"]);
        if let Some(output) = super::util::run_with_timeout(cmd, super::util::SLOW).await {
            let text = String::from_utf8_lossy(&output.stdout);
            for line in text.lines() {
                if line.contains("DNS Servers") || line.contains("Current DNS") {
                    if let Some(ip) = extract_ip(line) {
                        if ip != "127.0.0.53" && !servers.contains(&ip) {
                            servers.push(ip);
                        }
                    }
                }
            }
        }
    }

    servers
}

fn extract_ip(text: &str) -> Option<String> {
    for word in text.split_whitespace() {
        // Try IPv4 first
        let cleaned = word.trim_matches(|c: char| !c.is_ascii_digit() && c != '.');
        let parts: Vec<&str> = cleaned.split('.').collect();
        if parts.len() == 4 && parts.iter().all(|p| p.parse::<u8>().is_ok()) {
            return Some(cleaned.to_string());
        }

        // Try IPv6: must contain at least two colons and only hex digits/colons
        let trimmed = word.trim_matches(|c: char| !c.is_ascii_hexdigit() && c != ':');
        if trimmed.matches(':').count() >= 2
            && !trimmed.is_empty()
            && trimmed.chars().all(|c| c.is_ascii_hexdigit() || c == ':')
        {
            return Some(trimmed.to_string());
        }
    }
    None
}

/// Domains probed for the resolution verdict. Three independent zones with
/// distinct authoritative operators, so one slow/unlucky zone traversal can't
/// flip the verdict.
const TEST_DOMAINS: &[&str] = &["dns.google", "one.one.one.one", "example.com"];

async fn test_domain(domain: &'static str) -> DnsResolutionTest {
    let overall = Instant::now();

    // Each lookup is retried once after a short pause — a single transient
    // drop shouldn't count as a failed domain. The recorded time is the
    // *successful attempt's* time only, so a retry doesn't masquerade as a
    // slow resolver.
    let outcome = super::util::retry_probe(2, std::time::Duration::from_millis(500), || async {
        let start = Instant::now();
        let addrs =
            super::util::lookup_host_timeout(format!("{}:443", domain), super::util::RESOLVE)
                .await?;
        let elapsed = start.elapsed().as_secs_f64() * 1000.0;
        let ips: Vec<String> = addrs.into_iter().map(|a| a.ip().to_string()).collect();
        if ips.is_empty() {
            None
        } else {
            Some((elapsed, ips))
        }
    })
    .await;

    match outcome {
        Some((elapsed, ips)) => DnsResolutionTest {
            domain: domain.to_string(),
            resolved: true,
            resolution_time_ms: elapsed,
            resolved_ips: ips,
        },
        None => DnsResolutionTest {
            domain: domain.to_string(),
            resolved: false,
            resolution_time_ms: overall.elapsed().as_secs_f64() * 1000.0,
            resolved_ips: vec![],
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_result(domain: &str, resolved: bool, time_ms: f64) -> DnsResolutionTest {
        DnsResolutionTest {
            domain: domain.to_string(),
            resolved,
            resolution_time_ms: time_ms,
            resolved_ips: if resolved {
                vec!["192.0.2.1".to_string()]
            } else {
                vec![]
            },
        }
    }

    use super::super::DiagnosticStatus;

    #[test]
    fn all_fast_ok() {
        let tests = [
            test_result("dns.google", true, 80.0),
            test_result("one.one.one.one", true, 90.0),
            test_result("example.com", true, 110.0),
        ];
        let v = dns_verdict(&tests);
        assert_eq!(v.status, DiagnosticStatus::Ok);
        assert!(v.summary.contains("median"));
    }

    /// The de-flake regression test: one slow outlier among fast peers must
    /// not flip the verdict (the old single-domain check would have warned).
    #[test]
    fn single_slow_outlier_median_still_ok() {
        let tests = [
            test_result("dns.google", true, 1200.0),
            test_result("one.one.one.one", true, 80.0),
            test_result("example.com", true, 90.0),
        ];
        let v = dns_verdict(&tests);
        assert_eq!(v.status, DiagnosticStatus::Ok);
    }

    #[test]
    fn median_moderate_warns() {
        let tests = [
            test_result("dns.google", true, 250.0),
            test_result("one.one.one.one", true, 300.0),
            test_result("example.com", true, 220.0),
        ];
        let v = dns_verdict(&tests);
        assert_eq!(v.status, DiagnosticStatus::Warn);
        assert!(v.summary.contains("moderate"));
    }

    #[test]
    fn median_slow_warns() {
        let tests = [
            test_result("dns.google", true, 800.0),
            test_result("one.one.one.one", true, 900.0),
            test_result("example.com", true, 700.0),
        ];
        let v = dns_verdict(&tests);
        assert_eq!(v.status, DiagnosticStatus::Warn);
        assert!(v.summary.contains("slowly"));
    }

    #[test]
    fn partial_resolution_warns() {
        let tests = [
            test_result("dns.google", true, 80.0),
            test_result("one.one.one.one", false, 5000.0),
            test_result("example.com", true, 90.0),
        ];
        let v = dns_verdict(&tests);
        assert_eq!(v.status, DiagnosticStatus::Warn);
        assert!(v.summary.contains("2/3"));
    }

    #[test]
    fn all_failed_fails() {
        let tests = [
            test_result("dns.google", false, 5000.0),
            test_result("one.one.one.one", false, 5000.0),
            test_result("example.com", false, 5000.0),
        ];
        let v = dns_verdict(&tests);
        assert_eq!(v.status, DiagnosticStatus::Fail);
    }
}
