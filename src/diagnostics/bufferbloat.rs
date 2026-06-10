//! Real bufferbloat measurement.
//!
//! Idle baseline → sustained loaded-download phase → loaded-upload phase,
//! with ping bursts running CONCURRENTLY with the saturating traffic (the
//! old implementation slept 500ms into a single 25MB download and called
//! three pings "loaded latency" — an estimate, not a measurement). Graded
//! A+–F per direction by the loaded-vs-idle latency delta; overall grade is
//! the worse of the two.

use serde::Serialize;
use std::time::Duration;

use super::ping;
use crate::config::Config;

const TARGET: &str = "1.1.1.1";
const IDLE_PROBES: u32 = 5;
const DOWNLOAD_PROBES: u32 = 12;
const UPLOAD_PROBES: u32 = 8;
/// Concurrent saturating connections per phase.
const DOWNLOAD_STREAMS: usize = 4;
const UPLOAD_STREAMS: usize = 2;
const DOWNLOAD_URL: &str = "https://speed.cloudflare.com/__down?bytes=100000000";
const UPLOAD_URL: &str = "https://speed.cloudflare.com/__up";
const UPLOAD_CHUNK_BYTES: usize = 10_000_000;
/// Whole-module budget — the phases below are individually bounded, this is
/// the backstop.
const MODULE_BUDGET: Duration = Duration::from_secs(35);

#[derive(Debug, Clone, Serialize)]
pub struct BufferbloatResult {
    /// Idle median latency (kept field name for JSON compatibility).
    pub unloaded_latency_ms: f64,
    /// Download-phase median loaded latency (kept).
    pub loaded_latency_ms: Option<f64>,
    /// Overall grade — the worse of the per-direction grades (kept).
    pub grade: String,
    /// Download-direction latency delta (kept; same value as
    /// `download_bloat_ms`).
    pub bloat_ms: Option<f64>,
    pub description: String,
    // ── Additive fields (v3.4.0+) ──
    #[serde(skip_serializing_if = "Option::is_none")]
    pub download_bloat_ms: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub upload_loaded_latency_ms: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub upload_bloat_ms: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub download_grade: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub upload_grade: Option<String>,
    pub samples_idle: u32,
    pub samples_download: u32,
    pub samples_upload: u32,
}

pub async fn collect(config: &Config) -> Option<BufferbloatResult> {
    tokio::time::timeout(MODULE_BUDGET, collect_inner(config))
        .await
        .unwrap_or_default()
}

async fn collect_inner(config: &Config) -> Option<BufferbloatResult> {
    // Idle baseline.
    let idle_stats = run_burst(IDLE_PROBES).await?;
    let idle = median(&idle_stats.times_ms)?;

    // Honor --fast: the loaded phases deliberately saturate the link with
    // ~hundreds of MB of traffic — exactly what --fast asks to avoid. (The
    // old implementation silently downloaded 25 MB even under --fast.)
    if config.skip_speed {
        let grade = if idle < 20.0 {
            "A"
        } else if idle < 50.0 {
            "B"
        } else {
            "C"
        };
        return Some(BufferbloatResult {
            unloaded_latency_ms: idle,
            loaded_latency_ms: None,
            grade: grade.to_string(),
            bloat_ms: None,
            description: "Loaded latency not measured (run without --fast for full test)"
                .to_string(),
            download_bloat_ms: None,
            upload_loaded_latency_ms: None,
            upload_bloat_ms: None,
            download_grade: None,
            upload_grade: None,
            samples_idle: idle_stats.received(),
            samples_download: 0,
            samples_upload: 0,
        });
    }

    // Loaded download phase: saturate with concurrent streaming GETs while a
    // ping burst spans the window.
    let dl_stats = loaded_phase(Load::Download, DOWNLOAD_PROBES).await;
    // Loaded upload phase: concurrent looping POSTs.
    let ul_stats = loaded_phase(Load::Upload, UPLOAD_PROBES).await;

    let dl_median = dl_stats.as_ref().and_then(|s| median(&s.times_ms));
    let ul_median = ul_stats.as_ref().and_then(|s| median(&s.times_ms));

    let dl_bloat = dl_median.map(|m| m - idle);
    let ul_bloat = ul_median.map(|m| m - idle);

    let dl_grade = dl_bloat.map(grade_delta);
    let ul_grade = ul_bloat.map(grade_delta);
    let overall = worse_grade(dl_grade.unwrap_or("A+"), ul_grade.unwrap_or("A+"));

    let description = if dl_median.is_none() && ul_median.is_none() {
        "Loaded latency could not be measured".to_string()
    } else {
        describe(overall).to_string()
    };

    Some(BufferbloatResult {
        unloaded_latency_ms: idle,
        loaded_latency_ms: dl_median,
        grade: overall.to_string(),
        bloat_ms: dl_bloat,
        description,
        download_bloat_ms: dl_bloat,
        upload_loaded_latency_ms: ul_median,
        upload_bloat_ms: ul_bloat,
        download_grade: dl_grade.map(|g| g.to_string()),
        upload_grade: ul_grade.map(|g| g.to_string()),
        samples_idle: idle_stats.received(),
        samples_download: dl_stats.map(|s| s.received()).unwrap_or(0),
        samples_upload: ul_stats.map(|s| s.received()).unwrap_or(0),
    })
}

enum Load {
    Download,
    Upload,
}

/// Run a ping burst while concurrent tasks saturate the link in the given
/// direction. The saturating tasks are aborted as soon as the burst returns.
async fn loaded_phase(load: Load, probes: u32) -> Option<ping::PingStats> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .build()
        .ok()?;

    let mut tasks = Vec::new();
    let streams = match load {
        Load::Download => DOWNLOAD_STREAMS,
        Load::Upload => UPLOAD_STREAMS,
    };
    for _ in 0..streams {
        let client = client.clone();
        let handle = match load {
            Load::Download => tokio::spawn(async move {
                loop {
                    let Ok(mut resp) = client.get(DOWNLOAD_URL).send().await else {
                        continue;
                    };
                    while let Ok(Some(_chunk)) = resp.chunk().await {}
                }
            }),
            Load::Upload => tokio::spawn(async move {
                let payload = vec![0u8; UPLOAD_CHUNK_BYTES];
                loop {
                    let _ = client.post(UPLOAD_URL).body(payload.clone()).send().await;
                }
            }),
        };
        tasks.push(handle);
    }

    // Give the streams a moment to ramp, then ping through the load.
    tokio::time::sleep(Duration::from_millis(750)).await;
    let stats = run_burst(probes).await;

    for task in tasks {
        task.abort();
    }

    stats
}

async fn run_burst(count: u32) -> Option<ping::PingStats> {
    let stdout = ping::run_ping(TARGET, count).await?;
    let stats = ping::parse_ping(&stdout, count);
    if stats.received() == 0 {
        None
    } else {
        Some(stats)
    }
}

fn median(times: &[f64]) -> Option<f64> {
    if times.is_empty() {
        return None;
    }
    let mut sorted = times.to_vec();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    Some(sorted[sorted.len() / 2])
}

/// Grade a loaded-vs-idle latency delta (ms). Thresholds unchanged from the
/// original implementation.
fn grade_delta(delta_ms: f64) -> &'static str {
    if delta_ms < 5.0 {
        "A+"
    } else if delta_ms < 30.0 {
        "A"
    } else if delta_ms < 60.0 {
        "B"
    } else if delta_ms < 200.0 {
        "C"
    } else if delta_ms < 400.0 {
        "D"
    } else {
        "F"
    }
}

fn grade_rank(grade: &str) -> u8 {
    match grade {
        "A+" => 0,
        "A" => 1,
        "B" => 2,
        "C" => 3,
        "D" => 4,
        _ => 5,
    }
}

fn worse_grade<'a>(a: &'a str, b: &'a str) -> &'a str {
    if grade_rank(a) >= grade_rank(b) {
        a
    } else {
        b
    }
}

fn describe(grade: &str) -> &'static str {
    match grade {
        "A+" | "A" => "Excellent - minimal bufferbloat",
        "B" => "Good - minor bufferbloat",
        "C" => "Fair - moderate bufferbloat",
        "D" => "Poor - significant bufferbloat",
        _ => "Severe bufferbloat detected",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grade_delta_boundaries() {
        assert_eq!(grade_delta(4.9), "A+");
        assert_eq!(grade_delta(5.0), "A");
        assert_eq!(grade_delta(29.9), "A");
        assert_eq!(grade_delta(30.0), "B");
        assert_eq!(grade_delta(59.9), "B");
        assert_eq!(grade_delta(60.0), "C");
        assert_eq!(grade_delta(199.9), "C");
        assert_eq!(grade_delta(200.0), "D");
        assert_eq!(grade_delta(399.9), "D");
        assert_eq!(grade_delta(400.0), "F");
    }

    #[test]
    fn worse_grade_picks_worse() {
        assert_eq!(worse_grade("A+", "C"), "C");
        assert_eq!(worse_grade("F", "A"), "F");
        assert_eq!(worse_grade("B", "B"), "B");
    }

    #[test]
    fn median_is_middle_element() {
        assert_eq!(median(&[3.0, 1.0, 2.0]), Some(2.0));
        assert_eq!(median(&[]), None);
        assert_eq!(median(&[5.0]), Some(5.0));
    }
}
