use serde::Serialize;
use std::time::Duration;

use super::DiagnosticResult;

#[derive(Debug, Clone, Serialize)]
pub struct PortResult {
    pub port: u16,
    pub service: String,
    pub host: String,
    pub open: bool,
    pub latency_ms: Option<f64>,
}

const PORT_TESTS: &[(u16, &str, &str)] = &[
    (80, "HTTP", "1.1.1.1"),
    (443, "HTTPS", "1.1.1.1"),
    (53, "DNS", "8.8.8.8"),
    (22, "SSH", "github.com"),
];

pub async fn check() -> (DiagnosticResult, Vec<PortResult>) {
    let mut results = Vec::new();

    let mut handles = Vec::new();
    for (port, service, host) in PORT_TESTS {
        let port = *port;
        let service = service.to_string();
        let host = host.to_string();
        handles.push(tokio::spawn(async move {
            test_port(&host, port, &service).await
        }));
    }

    for handle in handles {
        if let Ok(result) = handle.await {
            results.push(result);
        }
    }

    let open = results.iter().filter(|r| r.open).count();
    let total = results.len();

    let result = if open == total {
        DiagnosticResult::ok("Ports", "All common ports open")
    } else if open == 0 {
        DiagnosticResult::fail("Ports", "All tested ports blocked")
    } else {
        let blocked: Vec<String> = results
            .iter()
            .filter(|r| !r.open)
            .map(|r| format!("{} ({})", r.service, r.port))
            .collect();
        DiagnosticResult::warn(
            "Ports",
            format!("Blocked: {}", blocked.join(", ")),
        )
    };

    (result, results)
}

async fn test_port(host: &str, port: u16, service: &str) -> PortResult {
    let addr = format!("{}:{}", host, port);
    let start = std::time::Instant::now();

    // Resolve hostname first if needed
    let addrs = match tokio::net::lookup_host(&addr).await {
        Ok(addrs) => addrs.collect::<Vec<_>>(),
        Err(_) => {
            return PortResult {
                port,
                service: service.to_string(),
                host: host.to_string(),
                open: false,
                latency_ms: None,
            };
        }
    };

    for addr in addrs {
        match tokio::time::timeout(
            Duration::from_secs(5),
            tokio::net::TcpStream::connect(addr),
        )
        .await
        {
            Ok(Ok(_stream)) => {
                let latency = start.elapsed().as_secs_f64() * 1000.0;
                return PortResult {
                    port,
                    service: service.to_string(),
                    host: host.to_string(),
                    open: true,
                    latency_ms: Some(latency),
                };
            }
            _ => continue,
        }
    }

    PortResult {
        port,
        service: service.to_string(),
        host: host.to_string(),
        open: false,
        latency_ms: None,
    }
}
