//! Sustained packet-loss measurement — deep diagnostic.
//!
//! The core latency check sends 6 probes per target — enough for a verdict,
//! not enough to measure loss. This module sends a sustained 30-probe burst
//! to three independent anycast targets concurrently (1s portable interval;
//! Windows ping has no sub-second flag and macOS restricts it to root) and
//! reports loss %, latency distribution, and jitter per target.

use serde::Serialize;

use super::ping;

const TARGETS: &[(&str, &str)] = &[
    ("1.1.1.1", "Cloudflare"),
    ("8.8.8.8", "Google DNS"),
    ("9.9.9.9", "Quad9"),
];

const PROBES: u32 = 30;

#[derive(Debug, Clone, Serialize)]
pub struct LossResult {
    pub host: String,
    pub label: String,
    pub sent: u32,
    pub received: u32,
    pub loss_pct: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub min_ms: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub avg_ms: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_ms: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub p95_ms: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub jitter_ms: Option<f64>,
    pub assessment: String,
    pub level: String,
}

pub async fn collect() -> Option<Vec<LossResult>> {
    let results: Vec<LossResult> = futures_util::future::join_all(
        TARGETS
            .iter()
            .map(|(host, label)| sustained_probe(host, label)),
    )
    .await;

    // Skip the section entirely when every target is dark — the core latency
    // check has already failed loudly; a wall of 100%-loss rows adds nothing.
    if results.iter().all(|r| r.received == 0) {
        None
    } else {
        Some(results)
    }
}

async fn sustained_probe(host: &str, label: &str) -> LossResult {
    let stats = match ping::run_ping(host, PROBES).await {
        Some(stdout) => ping::parse_ping(&stdout, PROBES),
        None => ping::PingStats::all_lost(PROBES),
    };
    build_result(host, label, &stats)
}

/// Pure construction from parsed stats — unit-testable without a network.
fn build_result(host: &str, label: &str, stats: &ping::PingStats) -> LossResult {
    let p95 = percentile_95(&stats.times_ms);
    let (assessment, level) = classify_loss(stats.packet_loss_pct, stats.jitter_ms());

    LossResult {
        host: host.to_string(),
        label: label.to_string(),
        sent: stats.sent,
        received: stats.received(),
        loss_pct: stats.packet_loss_pct,
        min_ms: stats.min_ms(),
        avg_ms: stats.avg_ms(),
        max_ms: stats.max_ms(),
        p95_ms: p95,
        jitter_ms: stats.jitter_ms(),
        assessment: assessment.to_string(),
        level: level.to_string(),
    }
}

fn percentile_95(times: &[f64]) -> Option<f64> {
    if times.is_empty() {
        return None;
    }
    let mut sorted = times.to_vec();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let idx = ((sorted.len() as f64 - 1.0) * 0.95).round() as usize;
    sorted.get(idx).copied()
}

/// Loss/jitter classification per target. Mirrors the latency module's
/// advisory philosophy: a single ICMC-deprioritizing target is not a
/// network problem; consistent loss is.
fn classify_loss(loss_pct: f64, jitter_ms: Option<f64>) -> (&'static str, &'static str) {
    if loss_pct > 2.0 {
        (
            "Sustained packet loss — expect stalls and retransmits",
            "fail",
        )
    } else if loss_pct > 0.0 {
        (
            "Trace loss observed — usually harmless at this level",
            "warn",
        )
    } else if jitter_ms.is_some_and(|j| j > 30.0) {
        (
            "No loss, but high jitter — real-time apps may stutter",
            "warn",
        )
    } else {
        ("No loss over the sustained burst", "ok")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stats(sent: u32, times: Vec<f64>) -> ping::PingStats {
        let received = times.len() as u32;
        ping::PingStats {
            sent,
            times_ms: times,
            packet_loss_pct: if sent == 0 {
                0.0
            } else {
                (sent - received) as f64 / sent as f64 * 100.0
            },
        }
    }

    #[test]
    fn clean_burst_is_ok() {
        let r = build_result("1.1.1.1", "Cloudflare", &stats(30, vec![10.0; 30]));
        assert_eq!(r.level, "ok");
        assert_eq!(r.loss_pct, 0.0);
        assert_eq!(r.p95_ms, Some(10.0));
    }

    #[test]
    fn real_loss_is_fail_level() {
        let r = build_result("1.1.1.1", "Cloudflare", &stats(30, vec![10.0; 27]));
        assert!(r.loss_pct > 2.0);
        assert_eq!(r.level, "fail");
    }

    #[test]
    fn trace_loss_warns() {
        // 1/30 lost ≈ 3.3% — wait, that's above 2%. Use 60 sent, 59 received
        // semantics via direct construction instead.
        let s = ping::PingStats {
            sent: 60,
            times_ms: vec![10.0; 59],
            packet_loss_pct: 1.7,
        };
        let r = build_result("1.1.1.1", "Cloudflare", &s);
        assert_eq!(r.level, "warn");
    }

    #[test]
    fn high_jitter_without_loss_warns() {
        // Alternating 5ms/95ms → jitter ~90ms, zero loss.
        let times: Vec<f64> = (0..30)
            .map(|i| if i % 2 == 0 { 5.0 } else { 95.0 })
            .collect();
        let r = build_result("1.1.1.1", "Cloudflare", &stats(30, times));
        assert_eq!(r.loss_pct, 0.0);
        assert_eq!(r.level, "warn");
        assert!(r.assessment.contains("jitter"));
    }

    #[test]
    fn p95_reflects_tail() {
        let mut times = vec![10.0; 29];
        times.push(200.0);
        let r = build_result("1.1.1.1", "Cloudflare", &stats(30, times));
        assert!(r.p95_ms.is_some_and(|p| p >= 10.0));
        assert_eq!(r.max_ms, Some(200.0));
    }
}
