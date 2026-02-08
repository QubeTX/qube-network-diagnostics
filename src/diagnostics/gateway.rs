use serde::Serialize;
use std::time::Instant;

use super::DiagnosticResult;

#[derive(Debug, Clone, Serialize)]
pub struct GatewayInfo {
    pub ip: String,
    pub reachable: bool,
    pub latency_ms: Option<f64>,
    pub interface: Option<String>,
}

pub async fn check() -> (DiagnosticResult, Option<GatewayInfo>) {
    let gateway_ip = match get_default_gateway() {
        Some(ip) => ip,
        None => {
            return (
                DiagnosticResult::fail("Gateway", "No default gateway detected"),
                None,
            );
        }
    };

    // Ping the gateway
    let (reachable, latency_ms) = ping_host(&gateway_ip).await;

    let info = GatewayInfo {
        ip: gateway_ip.clone(),
        reachable,
        latency_ms,
        interface: None,
    };

    let result = if reachable {
        let lat_str = latency_ms
            .map(|l| format!("{:.0}ms", l))
            .unwrap_or_else(|| "N/A".to_string());
        DiagnosticResult::ok("Gateway", format!("Reachable ({})", lat_str))
    } else {
        DiagnosticResult::fail("Gateway", format!("Gateway {} unreachable", gateway_ip))
    };

    (result, Some(info))
}

fn get_default_gateway() -> Option<String> {
    match default_net::get_default_gateway() {
        Ok(gw) => Some(gw.ip_addr.to_string()),
        Err(_) => None,
    }
}

async fn ping_host(host: &str) -> (bool, Option<f64>) {
    // Use platform-specific ping command
    let start = Instant::now();

    #[cfg(windows)]
    let result = tokio::process::Command::new("ping")
        .args(["-n", "1", "-w", "2000", host])
        .output()
        .await;

    #[cfg(unix)]
    let result = tokio::process::Command::new("ping")
        .args(["-c", "1", "-W", "2", host])
        .output()
        .await;

    match result {
        Ok(output) if output.status.success() => {
            let elapsed = start.elapsed();
            let text = String::from_utf8_lossy(&output.stdout);

            // Try to parse RTT from ping output
            let latency = parse_ping_latency(&text).unwrap_or(elapsed.as_secs_f64() * 1000.0);

            (true, Some(latency))
        }
        _ => (false, None),
    }
}

fn parse_ping_latency(output: &str) -> Option<f64> {
    // Windows: "Reply from ... time=1ms TTL=64"
    // Unix: "time=1.23 ms"
    for line in output.lines() {
        if let Some(pos) = line.find("time=") {
            let after = &line[pos + 5..];
            let num_str: String = after
                .chars()
                .take_while(|c| c.is_ascii_digit() || *c == '.' || *c == '<')
                .filter(|c| c.is_ascii_digit() || *c == '.')
                .collect();
            if let Ok(ms) = num_str.parse::<f64>() {
                return Some(ms);
            }
        }
        if let Some(pos) = line.find("time<") {
            let after = &line[pos + 5..];
            let num_str: String = after
                .chars()
                .take_while(|c| c.is_ascii_digit() || *c == '.')
                .collect();
            if let Ok(ms) = num_str.parse::<f64>() {
                return Some(ms);
            }
        }
    }
    None
}
