use serde::Serialize;
use std::time::Duration;

use super::DiagnosticResult;

/// Per-port outcome (additive field, v3.4.0+). `Unresolved` means no endpoint
/// for the port could be DNS-resolved — the port was never actually tested,
/// so it is excluded from the open/blocked denominator (a dead resolver is
/// the DNS check's finding, not a firewall finding).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum PortOutcome {
    Open,
    Blocked,
    Unresolved,
}

#[derive(Debug, Clone, Serialize)]
pub struct PortResult {
    pub port: u16,
    pub service: String,
    /// The endpoint that decided the outcome (the successful endpoint, else
    /// the primary).
    pub host: String,
    /// Kept for JSON backward compatibility — `outcome == Open`.
    pub open: bool,
    pub outcome: PortOutcome,
    /// TCP connect time only (resolution time excluded, v3.4.0+).
    pub latency_ms: Option<f64>,
}

/// Each port is tested against two endpoints on independent operators; the
/// port counts as open if either connects. Both endpoints refusing across
/// independent anycast providers is strong evidence of egress blocking
/// rather than a single service being down.
const PORT_TESTS: &[(u16, &str, &[&str])] = &[
    (80, "HTTP", &["1.1.1.1", "8.8.8.8"]),
    (443, "HTTPS", &["1.1.1.1", "8.8.8.8"]),
    (53, "DNS", &["8.8.8.8", "1.1.1.1"]),
    (22, "SSH", &["github.com", "gitlab.com"]),
];

const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
const RETRY_DELAY: Duration = Duration::from_millis(250);

pub async fn check() -> (DiagnosticResult, Vec<PortResult>) {
    let mut results = Vec::new();

    let mut handles = Vec::new();
    for (port, service, hosts) in PORT_TESTS {
        let port = *port;
        let service = service.to_string();
        let hosts: &'static [&'static str] = hosts;
        handles.push(tokio::spawn(async move {
            test_port_multi(hosts, port, &service).await
        }));
    }

    for handle in handles {
        if let Ok(result) = handle.await {
            results.push(result);
        }
    }

    let result = ports_verdict(&results);

    (result, results)
}

/// Pure verdict over the per-port results — unit-testable without a network.
///
/// Unresolved ports are excluded from the denominator; over the *tested*
/// ports: all open → Ok, some blocked → Warn listing them, none open → Fail.
fn ports_verdict(results: &[PortResult]) -> DiagnosticResult {
    let tested: Vec<&PortResult> = results
        .iter()
        .filter(|r| r.outcome != PortOutcome::Unresolved)
        .collect();
    let untested_note = if tested.len() < results.len() {
        " (SSH untested: DNS unavailable)"
    } else {
        ""
    };

    if tested.is_empty() {
        // Nothing could even be resolved — that's a DNS story, not a port
        // story, and the DNS check will have failed loudly already.
        return DiagnosticResult::fail("Ports", "No port tests could run (DNS unavailable)");
    }

    let open = tested
        .iter()
        .filter(|r| r.outcome == PortOutcome::Open)
        .count();

    if open == tested.len() {
        DiagnosticResult::ok("Ports", format!("All common ports open{}", untested_note))
    } else if open == 0 {
        DiagnosticResult::fail(
            "Ports",
            format!("All tested ports blocked{}", untested_note),
        )
    } else {
        let blocked: Vec<String> = tested
            .iter()
            .filter(|r| r.outcome == PortOutcome::Blocked)
            .map(|r| format!("{} ({})", r.service, r.port))
            .collect();
        DiagnosticResult::warn(
            "Ports",
            format!("Blocked: {}{}", blocked.join(", "), untested_note),
        )
    }
}

/// Test one port against its endpoints concurrently; first successful connect
/// wins. Each endpoint gets two attempts (transient SYN loss shouldn't mark a
/// port blocked).
async fn test_port_multi(hosts: &'static [&'static str], port: u16, service: &str) -> PortResult {
    let attempts =
        futures_util::future::join_all(hosts.iter().map(|host| test_endpoint(host, port))).await;

    // Prefer the first successful endpoint; else Blocked if any endpoint
    // resolved; else Unresolved.
    let mut decided: Option<(usize, EndpointOutcome)> = None;
    for (i, outcome) in attempts.iter().enumerate() {
        match outcome {
            EndpointOutcome::Open(latency) => {
                decided = Some((i, EndpointOutcome::Open(*latency)));
                break;
            }
            EndpointOutcome::Blocked if decided.is_none() => {
                decided = Some((i, EndpointOutcome::Blocked));
            }
            _ => {}
        }
    }

    match decided {
        Some((i, EndpointOutcome::Open(latency))) => PortResult {
            port,
            service: service.to_string(),
            host: hosts[i].to_string(),
            open: true,
            outcome: PortOutcome::Open,
            latency_ms: Some(latency),
        },
        Some((i, EndpointOutcome::Blocked)) => PortResult {
            port,
            service: service.to_string(),
            host: hosts[i].to_string(),
            open: false,
            outcome: PortOutcome::Blocked,
            latency_ms: None,
        },
        _ => PortResult {
            port,
            service: service.to_string(),
            host: hosts.first().copied().unwrap_or_default().to_string(),
            open: false,
            outcome: PortOutcome::Unresolved,
            latency_ms: None,
        },
    }
}

enum EndpointOutcome {
    /// Connected; payload is the TCP connect time in ms (resolve excluded).
    Open(f64),
    /// Resolved but no connection within the attempts.
    Blocked,
    /// DNS resolution failed — endpoint never tested.
    Unresolved,
}

async fn test_endpoint(host: &str, port: u16) -> EndpointOutcome {
    let addr = format!("{}:{}", host, port);

    // Resolve hostname first if needed (bounded so a dead resolver can't
    // hang). IP literals resolve locally and never take this branch's
    // failure path.
    let addrs = match super::util::lookup_host_timeout(addr, super::util::RESOLVE).await {
        Some(addrs) if !addrs.is_empty() => addrs,
        _ => return EndpointOutcome::Unresolved,
    };

    // Two attempts per endpoint: a transient SYN drop shouldn't read as a
    // blocked port. The clock starts after resolution so latency_ms is
    // connect-only.
    let connected = super::util::retry_probe(2, RETRY_DELAY, || {
        let addrs = addrs.clone();
        async move {
            for addr in addrs {
                let start = std::time::Instant::now();
                if let Ok(Ok(_stream)) =
                    tokio::time::timeout(CONNECT_TIMEOUT, tokio::net::TcpStream::connect(addr))
                        .await
                {
                    return Some(start.elapsed().as_secs_f64() * 1000.0);
                }
            }
            None
        }
    })
    .await;

    match connected {
        Some(latency) => EndpointOutcome::Open(latency),
        None => EndpointOutcome::Blocked,
    }
}

#[cfg(test)]
mod tests {
    use super::super::DiagnosticStatus;
    use super::*;

    fn port(service: &str, port_no: u16, outcome: PortOutcome) -> PortResult {
        PortResult {
            port: port_no,
            service: service.to_string(),
            host: "192.0.2.1".to_string(),
            open: outcome == PortOutcome::Open,
            outcome,
            latency_ms: if outcome == PortOutcome::Open {
                Some(5.0)
            } else {
                None
            },
        }
    }

    #[test]
    fn all_open_ok() {
        let results = [
            port("HTTP", 80, PortOutcome::Open),
            port("HTTPS", 443, PortOutcome::Open),
            port("DNS", 53, PortOutcome::Open),
            port("SSH", 22, PortOutcome::Open),
        ];
        assert_eq!(ports_verdict(&results).status, DiagnosticStatus::Ok);
    }

    #[test]
    fn one_blocked_warns_and_lists_it() {
        let results = [
            port("HTTP", 80, PortOutcome::Open),
            port("HTTPS", 443, PortOutcome::Blocked),
            port("DNS", 53, PortOutcome::Open),
            port("SSH", 22, PortOutcome::Open),
        ];
        let v = ports_verdict(&results);
        assert_eq!(v.status, DiagnosticStatus::Warn);
        assert!(v.summary.contains("HTTPS (443)"));
    }

    #[test]
    fn all_blocked_fails() {
        let results = [
            port("HTTP", 80, PortOutcome::Blocked),
            port("HTTPS", 443, PortOutcome::Blocked),
            port("DNS", 53, PortOutcome::Blocked),
            port("SSH", 22, PortOutcome::Blocked),
        ];
        assert_eq!(ports_verdict(&results).status, DiagnosticStatus::Fail);
    }

    /// SSH with dead DNS is "untested", not "blocked" — it must not drag an
    /// otherwise-open verdict down.
    #[test]
    fn unresolved_excluded_from_denominator_ok() {
        let results = [
            port("HTTP", 80, PortOutcome::Open),
            port("HTTPS", 443, PortOutcome::Open),
            port("DNS", 53, PortOutcome::Open),
            port("SSH", 22, PortOutcome::Unresolved),
        ];
        let v = ports_verdict(&results);
        assert_eq!(v.status, DiagnosticStatus::Ok);
        assert!(v.summary.contains("SSH untested"));
    }

    #[test]
    fn unresolved_plus_all_blocked_still_fails() {
        let results = [
            port("HTTP", 80, PortOutcome::Blocked),
            port("HTTPS", 443, PortOutcome::Blocked),
            port("DNS", 53, PortOutcome::Blocked),
            port("SSH", 22, PortOutcome::Unresolved),
        ];
        assert_eq!(ports_verdict(&results).status, DiagnosticStatus::Fail);
    }

    #[test]
    fn open_via_alternate_endpoint_counts_open() {
        // The outcome enum is already endpoint-merged by test_port_multi;
        // this pins the verdict layer's contract that Open is Open no matter
        // which endpoint answered.
        let results = [
            port("HTTP", 80, PortOutcome::Open),
            port("HTTPS", 443, PortOutcome::Open),
            port("DNS", 53, PortOutcome::Open),
            port("SSH", 22, PortOutcome::Open),
        ];
        let v = ports_verdict(&results);
        assert_eq!(v.status, DiagnosticStatus::Ok);
    }
}
