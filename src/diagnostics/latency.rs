use serde::Serialize;

use super::ping;
use super::DiagnosticResult;

#[derive(Debug, Clone, Serialize)]
pub struct LatencyResult {
    pub host: String,
    pub label: String,
    pub reachable: bool,
    pub min_ms: Option<f64>,
    pub avg_ms: Option<f64>,
    pub max_ms: Option<f64>,
    pub jitter_ms: Option<f64>,
    pub packet_loss: f64,
}

const TARGETS: &[(&str, &str)] = &[
    ("1.1.1.1", "Cloudflare"),
    ("8.8.8.8", "Google DNS"),
    ("208.67.222.222", "OpenDNS"),
];

/// Probes per target. Six samples give the average enough support that one
/// slow reply doesn't flip the verdict; the per-command budget scales with
/// the count via `util::ping_budget` (a 6-probe burst can take ~12-16s on a
/// degraded link, beyond the old fixed 10s cap).
const SAMPLES: u32 = 6;

pub async fn check() -> (DiagnosticResult, Vec<LatencyResult>) {
    // Ping all targets concurrently. `join_all` preserves input order, so the
    // reachable/avg computations below are identical to the sequential version.
    let results: Vec<LatencyResult> = futures_util::future::join_all(
        TARGETS
            .iter()
            .map(|(host, label)| ping_multiple(host, label, SAMPLES)),
    )
    .await;

    let result = latency_verdict(&results);

    (result, results)
}

/// Pure verdict over the per-target results — unit-testable without a network.
///
/// - 0 reachable → Fail
/// - exactly 1 reachable → Warn (partial reachability) regardless of that
///   target's latency — a single target's path latency must not set the
///   message when the other two are dark
/// - ≥2 reachable → average-based thresholds (>200ms high, >100ms moderate),
///   plus the partial-reachability Warn when one target is still missing
fn latency_verdict(results: &[LatencyResult]) -> DiagnosticResult {
    let reachable = results.iter().filter(|r| r.reachable).count();
    let total = results.len();

    if reachable == 0 {
        return DiagnosticResult::fail("Latency", "All endpoints unreachable");
    }

    if reachable == 1 {
        if let Some(only) = results.iter().find(|r| r.reachable) {
            let lat = only
                .avg_ms
                .map(|l| format!("{:.0}ms", l))
                .unwrap_or_else(|| "N/A".to_string());
            return DiagnosticResult::warn(
                "Latency",
                format!(
                    "Only 1/{} endpoints reachable ({} to {})",
                    total, lat, only.label
                ),
            );
        }
    }

    let avg_latency: f64 = results.iter().filter_map(|r| r.avg_ms).sum::<f64>() / reachable as f64;

    if avg_latency > 200.0 {
        DiagnosticResult::warn(
            "Latency",
            format!("High latency (~{:.0}ms avg)", avg_latency),
        )
    } else if avg_latency > 100.0 {
        DiagnosticResult::warn(
            "Latency",
            format!("Moderate latency (~{:.0}ms avg)", avg_latency),
        )
    } else if reachable < total {
        DiagnosticResult::warn(
            "Latency",
            format!("{}/{} endpoints reachable", reachable, total),
        )
    } else {
        DiagnosticResult::ok("Latency", "Low latency to all endpoints")
    }
}

async fn ping_multiple(host: &str, label: &str, count: u32) -> LatencyResult {
    match ping::run_ping(host, count).await {
        Some(stdout) => {
            let stats = ping::parse_ping(&stdout, count);
            if stats.received() == 0 {
                unreachable_result(host, label)
            } else {
                LatencyResult {
                    host: host.to_string(),
                    label: label.to_string(),
                    reachable: true,
                    min_ms: stats.min_ms(),
                    avg_ms: stats.avg_ms(),
                    max_ms: stats.max_ms(),
                    jitter_ms: stats.jitter_ms(),
                    packet_loss: stats.packet_loss_pct,
                }
            }
        }
        None => unreachable_result(host, label),
    }
}

fn unreachable_result(host: &str, label: &str) -> LatencyResult {
    LatencyResult {
        host: host.to_string(),
        label: label.to_string(),
        reachable: false,
        min_ms: None,
        avg_ms: None,
        max_ms: None,
        jitter_ms: None,
        packet_loss: 100.0,
    }
}

#[cfg(test)]
mod tests {
    use super::super::DiagnosticStatus;
    use super::*;

    fn target(label: &str, reachable: bool, avg_ms: Option<f64>) -> LatencyResult {
        LatencyResult {
            host: "192.0.2.1".to_string(),
            label: label.to_string(),
            reachable,
            min_ms: avg_ms,
            avg_ms,
            max_ms: avg_ms,
            jitter_ms: None,
            packet_loss: if reachable { 0.0 } else { 100.0 },
        }
    }

    #[test]
    fn all_reachable_low_ok() {
        let results = [
            target("Cloudflare", true, Some(10.0)),
            target("Google DNS", true, Some(12.0)),
            target("OpenDNS", true, Some(15.0)),
        ];
        assert_eq!(latency_verdict(&results).status, DiagnosticStatus::Ok);
    }

    #[test]
    fn all_reachable_moderate_warns() {
        let results = [
            target("Cloudflare", true, Some(120.0)),
            target("Google DNS", true, Some(130.0)),
            target("OpenDNS", true, Some(140.0)),
        ];
        let v = latency_verdict(&results);
        assert_eq!(v.status, DiagnosticStatus::Warn);
        assert!(v.summary.contains("Moderate"));
    }

    #[test]
    fn all_reachable_high_warns() {
        let results = [
            target("Cloudflare", true, Some(250.0)),
            target("Google DNS", true, Some(260.0)),
            target("OpenDNS", true, Some(270.0)),
        ];
        let v = latency_verdict(&results);
        assert_eq!(v.status, DiagnosticStatus::Warn);
        assert!(v.summary.contains("High"));
    }

    /// The dominance regression test: with exactly one reachable target, its
    /// latency must not produce a "High latency" verdict — the partial
    /// reachability is the story.
    #[test]
    fn one_reachable_high_latency_reports_partial_not_high() {
        let results = [
            target("Cloudflare", true, Some(250.0)),
            target("Google DNS", false, None),
            target("OpenDNS", false, None),
        ];
        let v = latency_verdict(&results);
        assert_eq!(v.status, DiagnosticStatus::Warn);
        assert!(
            v.summary.contains("Only 1/3"),
            "summary should report partial reachability, got: {}",
            v.summary
        );
        assert!(!v.summary.contains("High latency"));
    }

    #[test]
    fn two_of_three_low_avg_warns_partial() {
        let results = [
            target("Cloudflare", true, Some(10.0)),
            target("Google DNS", true, Some(12.0)),
            target("OpenDNS", false, None),
        ];
        let v = latency_verdict(&results);
        assert_eq!(v.status, DiagnosticStatus::Warn);
        assert!(v.summary.contains("2/3"));
    }

    #[test]
    fn none_reachable_fails() {
        let results = [
            target("Cloudflare", false, None),
            target("Google DNS", false, None),
            target("OpenDNS", false, None),
        ];
        assert_eq!(latency_verdict(&results).status, DiagnosticStatus::Fail);
    }
}
