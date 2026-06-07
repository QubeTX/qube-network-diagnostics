use serde::Serialize;
use std::time::Instant;

use super::DiagnosticResult;

#[derive(Debug, Clone, Serialize)]
pub struct DnsInfo {
    pub servers: Vec<DnsServer>,
    pub resolution_test: Option<DnsResolutionTest>,
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

    // Test DNS resolution
    let resolution = test_dns_resolution().await;

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
        resolution_test: resolution.clone(),
    };

    let result = match resolution {
        Some(ref test) if test.resolved => {
            let time_ms = test.resolution_time_ms;
            if time_ms > 500.0 {
                DiagnosticResult::warn("DNS", format!("Resolving slowly ({:.0}ms)", time_ms))
            } else if time_ms > 200.0 {
                DiagnosticResult::warn(
                    "DNS",
                    format!("Resolving with moderate latency ({:.0}ms)", time_ms),
                )
            } else {
                DiagnosticResult::ok("DNS", format!("Resolving normally ({:.0}ms)", time_ms))
            }
        }
        _ => DiagnosticResult::fail("DNS", "DNS resolution failed"),
    };

    (result, Some(info))
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

async fn test_dns_resolution() -> Option<DnsResolutionTest> {
    let domain = "dns.google";
    let start = Instant::now();

    // Use system DNS resolution (bounded so a black-holed resolver can't hang).
    match super::util::lookup_host_timeout(format!("{}:443", domain), super::util::RESOLVE).await {
        Some(addrs) => {
            let elapsed = start.elapsed().as_secs_f64() * 1000.0;
            let ips: Vec<String> = addrs.into_iter().map(|a| a.ip().to_string()).collect();
            Some(DnsResolutionTest {
                domain: domain.to_string(),
                resolved: !ips.is_empty(),
                resolution_time_ms: elapsed,
                resolved_ips: ips,
            })
        }
        None => Some(DnsResolutionTest {
            domain: domain.to_string(),
            resolved: false,
            resolution_time_ms: start.elapsed().as_secs_f64() * 1000.0,
            resolved_ips: vec![],
        }),
    }
}
