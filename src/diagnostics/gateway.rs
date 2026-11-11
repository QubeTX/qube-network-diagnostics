use serde::Serialize;

use super::ping::{self, PingStats};
use super::DiagnosticResult;

#[derive(Debug, Clone, Serialize)]
pub struct GatewayInfo {
    pub ip: String,
    pub reachable: bool,
    pub latency_ms: Option<f64>,
    pub interface: Option<String>,
    /// Probes sent across both bursts (additive field, v3.4.0+).
    pub packets_sent: u32,
    /// Replies received across both bursts (additive field, v3.4.0+).
    pub packets_received: u32,
}

/// First burst size. If the whole first burst is lost, a second burst of the
/// same size runs after a short pause (covers ARP warm-up and Wi-Fi
/// power-save wakeup), so Fail requires 0/6 replies — a single dropped packet
/// can no longer flip the verdict.
const BURST: u32 = 3;
const SECOND_BURST_DELAY: std::time::Duration = std::time::Duration::from_millis(500);

pub async fn check() -> (DiagnosticResult, Option<GatewayInfo>) {
    let gateway_ip = match get_default_gateway().await {
        Some(ip) => ip,
        None => {
            return (
                DiagnosticResult::fail("Gateway", "No default gateway detected"),
                None,
            );
        }
    };

    let stats = ping_gateway(&gateway_ip).await;

    let info = GatewayInfo {
        ip: gateway_ip.clone(),
        reachable: stats.received() > 0,
        latency_ms: stats.avg_ms(),
        interface: None,
        packets_sent: stats.sent,
        packets_received: stats.received(),
    };

    let result = gateway_verdict(&gateway_ip, &stats);

    (result, Some(info))
}

/// Pure verdict over a ping burst — unit-testable without a network.
///
/// - every probe replied → Ok
/// - some probes replied → Warn with the loss percentage
/// - zero replies (across both bursts) → Fail
fn gateway_verdict(ip: &str, stats: &PingStats) -> DiagnosticResult {
    let received = stats.received();
    if received == 0 {
        return DiagnosticResult::fail("Gateway", format!("Gateway {} unreachable", ip));
    }

    let lat_str = stats
        .avg_ms()
        .map(|l| format!("{:.0}ms", l))
        .unwrap_or_else(|| "N/A".to_string());

    if received == stats.sent {
        DiagnosticResult::ok("Gateway", format!("Reachable ({})", lat_str))
    } else {
        let loss = if stats.sent == 0 {
            0.0
        } else {
            (stats.sent - received) as f64 / stats.sent as f64 * 100.0
        };
        DiagnosticResult::warn(
            "Gateway",
            format!("Reachable with {:.0}% packet loss ({} avg)", loss, lat_str),
        )
    }
}

/// Burst of [`BURST`] pings; if every probe is lost, pause briefly and try a
/// second burst, merging the stats (so `sent` reflects both bursts).
async fn ping_gateway(host: &str) -> PingStats {
    let first = match ping::run_ping(host, BURST).await {
        Some(stdout) => ping::parse_ping(&stdout, BURST),
        None => PingStats::all_lost(BURST),
    };

    if first.received() > 0 {
        return first;
    }

    tokio::time::sleep(SECOND_BURST_DELAY).await;

    let second = match ping::run_ping(host, BURST).await {
        Some(stdout) => ping::parse_ping(&stdout, BURST),
        None => PingStats::all_lost(BURST),
    };

    first.merged_with(second)
}

async fn get_default_gateway() -> Option<String> {
    // `default_net::get_default_gateway` is a synchronous, potentially-blocking
    // syscall; run it off the async runtime so it can't stall the executor.
    // A JoinError (panic in the blocking task) falls back to None, identical
    // to the pre-existing Err path.
    tokio::task::spawn_blocking(|| {
        default_net::get_default_gateway()
            .ok()
            .map(|gw| gw.ip_addr.to_string())
    })
    .await
    .unwrap_or(None)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stats(sent: u32, times: &[f64]) -> PingStats {
        PingStats {
            sent,
            times_ms: times.to_vec(),
            packet_loss_pct: if sent == 0 {
                0.0
            } else {
                (sent as usize - times.len()) as f64 / sent as f64 * 100.0
            },
        }
    }

    #[test]
    fn all_replies_ok() {
        let result = gateway_verdict("192.168.1.1", &stats(3, &[1.0, 1.2, 0.9]));
        assert_eq!(result.status, super::super::DiagnosticStatus::Ok);
    }

    #[test]
    fn partial_loss_warns_with_pct() {
        let result = gateway_verdict("192.168.1.1", &stats(6, &[1.0, 1.2]));
        assert_eq!(result.status, super::super::DiagnosticStatus::Warn);
        assert!(
            result.summary.contains("loss"),
            "summary: {}",
            result.summary
        );
        assert!(
            result.summary.contains("67%"),
            "summary: {}",
            result.summary
        );
    }

    #[test]
    fn zero_replies_fails() {
        let result = gateway_verdict("192.168.1.1", &stats(6, &[]));
        assert_eq!(result.status, super::super::DiagnosticStatus::Fail);
        assert!(result.summary.contains("unreachable"));
    }

    #[test]
    fn ok_summary_includes_latency() {
        let result = gateway_verdict("10.0.0.1", &stats(3, &[2.0, 2.0, 2.0]));
        assert!(
            result.summary.contains("2ms"),
            "summary: {}",
            result.summary
        );
    }
}
