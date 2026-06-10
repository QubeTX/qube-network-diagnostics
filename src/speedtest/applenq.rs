//! Apple networkQuality capacity provider.
//!
//! Speaks the load-generation half of the IETF IPPM responsiveness
//! methodology (draft-ietf-ippm-responsiveness) against Apple's public
//! mensura.cdn-apple.com infrastructure: plain HTTPS GETs of a very large
//! object and POSTs to an upload sink, run on several parallel connections to
//! saturate the link. Only capacity is measured here — the RPM/responsiveness
//! metric is out of scope.
//!
//! Third-party clients against Apple's infrastructure are documented accepted
//! practice: the network-quality GitHub org (which Apple co-maintains
//! alongside its open-source reference server) ships goresponsiveness with
//! instructions for testing against `mensura.cdn-apple.com`. Load here is
//! conservative (4 connections, bounded duration) and failures are silent per
//! the provider convention; `--skip-apple` disables the provider entirely.
//!
//! Why not Ookla/Speedtest.net instead? Evaluated and excluded: Ookla's EULA
//! permits only personal, non-commercial use through the official CLI, with
//! no third-party protocol authorization (speedtest.net/about/eula,
//! speedtest.net/about/terms) — unusable for an open-source tool that ships
//! corporate installers. Apple's edge (aaplimg.com) is also a distinct CDN
//! family from Cloudflare/M-Lab/Netflix/LibreSpeed, so it adds independent
//! signal to the cross-provider merge.

use super::{statistics, BandwidthSamples, Phase, ProviderResult, SpeedTestConfig, TestDuration};
use reqwest::Client;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

const CONFIG_URL: &str = "https://mensura.cdn-apple.com/api/v1/gm/config";

/// Parallel connections per direction — the methodology saturates with a
/// handful of connections; 4 matches the reference clients' starting load.
const CONNECTIONS: usize = 4;

/// Upload POST body size per request (2 MB, matching the Cloudflare loop).
const UPLOAD_CHUNK_BYTES: usize = 2_000_000;

/// Latency probes against the small object (first two discarded as warmup).
const LATENCY_PROBES: usize = 10;
const LATENCY_WARMUP_DISCARD: usize = 2;

/// Aggregate sampling tick across the parallel connections.
const SAMPLE_INTERVAL: Duration = Duration::from_millis(500);

/// Default auto-mode durations (seconds): mirrors the Cloudflare split.
const AUTO_DOWNLOAD_SECS: u64 = 15;
const AUTO_UPLOAD_SECS: u64 = 10;

const MIN_REQUEST_TIMEOUT: Duration = Duration::from_secs(1);

fn remaining_budget(deadline: Instant) -> Duration {
    deadline
        .saturating_duration_since(Instant::now())
        .max(MIN_REQUEST_TIMEOUT)
}

/// Run the Apple networkQuality capacity test.
pub async fn run<F>(config: &SpeedTestConfig, progress: F) -> ProviderResult
where
    F: Fn(Phase, f64) + Send + Sync,
{
    match run_inner(config, &progress).await {
        Ok(result) => result,
        Err(e) => error_result(e.to_string()),
    }
}

async fn run_inner<F>(config: &SpeedTestConfig, progress: &F) -> Result<ProviderResult, String>
where
    F: Fn(Phase, f64) + Send + Sync,
{
    let client = Client::builder()
        .timeout(Duration::from_secs(120))
        .build()
        .map_err(|e| format!("HTTP client error: {e}"))?;

    // ── Config discovery ─────────────────────────────────────────────
    progress(Phase::AnqDiscovery, 0.0);

    let body: serde_json::Value = client
        .get(CONFIG_URL)
        .timeout(Duration::from_secs(15))
        .send()
        .await
        .map_err(|e| format!("Apple networkQuality config fetch failed: {e}"))?
        .json()
        .await
        .map_err(|e| format!("Apple networkQuality config parse error: {e}"))?;

    let parsed = parse_config(&body)?;

    progress(Phase::AnqDiscovery, 0.5);

    // ── Latency: small-object probes (warmup discarded, min + jitter) ─
    let mut rtts: Vec<f64> = Vec::with_capacity(LATENCY_PROBES);
    for _ in 0..LATENCY_PROBES {
        let start = Instant::now();
        if let Ok(resp) = client
            .get(&parsed.small_url)
            .timeout(Duration::from_secs(5))
            .send()
            .await
        {
            // Drain the (tiny) body so the next probe reuses the connection.
            let _ = resp.bytes().await;
            rtts.push(start.elapsed().as_secs_f64() * 1000.0);
        }
    }
    let warmup_skip = LATENCY_WARMUP_DISCARD.min(rtts.len());
    let trimmed = &rtts[warmup_skip..];
    let ping_ms = trimmed
        .iter()
        .copied()
        .min_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let jitter_ms = if trimmed.len() >= 2 {
        Some(statistics::jitter_rfc3550(trimmed))
    } else {
        None
    };

    progress(Phase::AnqDiscovery, 1.0);

    // ── Duration budget ──────────────────────────────────────────────
    let (dl_secs, ul_secs) = match &config.duration {
        TestDuration::Seconds(s) => (*s, *s),
        TestDuration::Auto => (AUTO_DOWNLOAD_SECS, AUTO_UPLOAD_SECS),
    };

    // ── Download: parallel streaming GETs of the large object ───────
    progress(Phase::AnqDownload, 0.0);
    let (dl_samples, dl_bytes, dl_elapsed) = saturate(
        &client,
        SaturateSpec::Download {
            url: parsed.large_url.clone(),
        },
        Duration::from_secs(dl_secs),
        |frac| progress(Phase::AnqDownload, frac),
    )
    .await;
    progress(Phase::AnqDownload, 1.0);

    // ── Upload: parallel POSTs to the slurp sink ─────────────────────
    progress(Phase::AnqUpload, 0.0);
    let (ul_samples, ul_bytes, ul_elapsed) = saturate(
        &client,
        SaturateSpec::Upload {
            url: parsed.upload_url.clone(),
        },
        Duration::from_secs(ul_secs),
        |frac| progress(Phase::AnqUpload, frac),
    )
    .await;
    progress(Phase::AnqUpload, 1.0);

    if dl_samples.is_empty() && ul_samples.is_empty() {
        return Err("no successful transfers".to_string());
    }

    let download_mbps = if dl_samples.is_empty() {
        None
    } else {
        Some(statistics::accurate_bandwidth(&dl_samples))
    };
    let upload_mbps = if ul_samples.is_empty() {
        None
    } else {
        Some(statistics::accurate_upload_bandwidth(&ul_samples))
    };

    Ok(ProviderResult {
        provider: "Apple networkQuality".to_string(),
        server: parsed.test_endpoint,
        location: None,
        ping_ms,
        jitter_ms,
        download_mbps,
        upload_mbps,
        download_bytes: dl_bytes,
        upload_bytes: ul_bytes,
        download_duration_s: dl_elapsed,
        upload_duration_s: ul_elapsed,
        packet_loss_pct: None,
        error: None,
        bandwidth_samples: Some(BandwidthSamples {
            download: dl_samples,
            upload: ul_samples,
        }),
    })
}

struct ParsedConfig {
    large_url: String,
    small_url: String,
    upload_url: String,
    test_endpoint: String,
}

/// Parse the /api/v1/gm/config JSON (shape specified by the responsiveness
/// draft: `urls.{large_https_download_url, small_https_download_url,
/// https_upload_url}` plus the serving `test_endpoint`).
fn parse_config(body: &serde_json::Value) -> Result<ParsedConfig, String> {
    let urls = &body["urls"];
    let large_url = urls["large_https_download_url"]
        .as_str()
        .ok_or("Apple networkQuality config: missing large download URL")?
        .to_string();
    let small_url = urls["small_https_download_url"]
        .as_str()
        .ok_or("Apple networkQuality config: missing small download URL")?
        .to_string();
    let upload_url = urls["https_upload_url"]
        .as_str()
        .ok_or("Apple networkQuality config: missing upload URL")?
        .to_string();
    let test_endpoint = body["test_endpoint"]
        .as_str()
        .unwrap_or("mensura.cdn-apple.com")
        .to_string();
    Ok(ParsedConfig {
        large_url,
        small_url,
        upload_url,
        test_endpoint,
    })
}

enum SaturateSpec {
    Download { url: String },
    Upload { url: String },
}

/// Saturate the link with [`CONNECTIONS`] parallel workers for `budget`,
/// sampling the aggregate byte counter every 500ms. Returns (samples, total
/// bytes, elapsed seconds). Per-request errors are silently retried by the
/// worker loops (provider convention).
async fn saturate<F>(
    client: &Client,
    spec: SaturateSpec,
    budget: Duration,
    progress: F,
) -> (Vec<f64>, u64, f64)
where
    F: Fn(f64),
{
    let start = Instant::now();
    let deadline = start + budget;
    let counter = Arc::new(AtomicU64::new(0));

    let mut handles = Vec::new();
    for _ in 0..CONNECTIONS {
        let client = client.clone();
        let counter = counter.clone();
        let handle = match &spec {
            SaturateSpec::Download { url } => {
                let url = url.clone();
                tokio::spawn(async move {
                    while Instant::now() < deadline {
                        let Ok(resp) = client
                            .get(&url)
                            .timeout(remaining_budget(deadline))
                            .send()
                            .await
                        else {
                            continue;
                        };
                        if !resp.status().is_success() {
                            continue;
                        }
                        // Stream the (8 GB+) object chunk by chunk; one
                        // request typically spans the whole phase.
                        let mut resp = resp;
                        while let Ok(Some(chunk)) = resp.chunk().await {
                            counter.fetch_add(chunk.len() as u64, Ordering::Relaxed);
                            if Instant::now() >= deadline {
                                break;
                            }
                        }
                    }
                })
            }
            SaturateSpec::Upload { url } => {
                let url = url.clone();
                tokio::spawn(async move {
                    let payload = vec![0u8; UPLOAD_CHUNK_BYTES];
                    while Instant::now() < deadline {
                        match client
                            .post(&url)
                            .body(payload.clone())
                            .timeout(remaining_budget(deadline).min(Duration::from_secs(30)))
                            .send()
                            .await
                        {
                            Ok(resp) if resp.status().is_success() => {
                                counter.fetch_add(UPLOAD_CHUNK_BYTES as u64, Ordering::Relaxed);
                            }
                            _ => {}
                        }
                    }
                })
            }
        };
        handles.push(handle);
    }

    // Aggregate sampler.
    let mut samples: Vec<f64> = Vec::new();
    let mut last_total: u64 = 0;
    let mut last_at = Instant::now();
    let mut sampler = tokio::time::interval(SAMPLE_INTERVAL);
    sampler.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    sampler.reset();

    let join_deadline = tokio::time::Instant::from_std(deadline + Duration::from_secs(35));
    let mut joined = futures_util::future::join_all(handles);
    loop {
        tokio::select! {
            _ = &mut joined => break,
            _ = sampler.tick() => {
                let total = counter.load(Ordering::Relaxed);
                let now = Instant::now();
                let dt = now.duration_since(last_at).as_secs_f64();
                let db = total.saturating_sub(last_total);
                if dt > 0.1 && db > 0 {
                    samples.push(db as f64 * 8.0 / (dt * 1_000_000.0));
                }
                last_total = total;
                last_at = now;
                progress((start.elapsed().as_secs_f64() / budget.as_secs_f64()).min(0.99));
            }
            _ = tokio::time::sleep_until(join_deadline) => break,
        }
    }

    (
        samples,
        counter.load(Ordering::Relaxed),
        start.elapsed().as_secs_f64(),
    )
}

fn error_result(msg: String) -> ProviderResult {
    ProviderResult {
        provider: "Apple networkQuality".to_string(),
        server: "unknown".to_string(),
        location: None,
        ping_ms: None,
        jitter_ms: None,
        download_mbps: None,
        upload_mbps: None,
        download_bytes: 0,
        upload_bytes: 0,
        download_duration_s: 0.0,
        upload_duration_s: 0.0,
        packet_loss_pct: None,
        error: Some(msg),
        bandwidth_samples: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Shape verified against a live fetch of the config endpoint.
    const CONFIG_FIXTURE: &str = r#"{
        "version": 1,
        "test_endpoint": "usdal2-edge-fx-030.aaplimg.com",
        "urls": {
            "small_https_download_url": "https://mensura.cdn-apple.com/api/v1/gm/small",
            "large_https_download_url": "https://mensura.cdn-apple.com/api/v1/gm/large",
            "https_upload_url": "https://mensura.cdn-apple.com/api/v1/gm/slurp"
        }
    }"#;

    #[test]
    fn config_fixture_parses() {
        let body: serde_json::Value = serde_json::from_str(CONFIG_FIXTURE).unwrap();
        let parsed = parse_config(&body).unwrap();
        assert_eq!(parsed.test_endpoint, "usdal2-edge-fx-030.aaplimg.com");
        assert!(parsed.large_url.ends_with("/large"));
        assert!(parsed.small_url.ends_with("/small"));
        assert!(parsed.upload_url.ends_with("/slurp"));
    }

    #[test]
    fn config_missing_urls_is_error() {
        let body: serde_json::Value = serde_json::from_str(r#"{"version":1}"#).unwrap();
        assert!(parse_config(&body).is_err());
    }
}
