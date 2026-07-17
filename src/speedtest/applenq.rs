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

use super::{
    statistics, BandwidthSamples, LatencyStats, Phase, ProviderAvailability, ProviderResult,
    SpeedTestConfig, TestDuration,
};
use reqwest::Client;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

const CONFIG_URL: &str = "https://mensura.cdn-apple.com/api/v1/gm/config";

/// Cache-busting variant of a URL (appends a unique nonce query param).
fn cache_bust(url: &str) -> String {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let sep = if url.contains('?') { '&' } else { '?' };
    format!("{url}{sep}nocache={nanos}")
}

/// Parallel connections per direction — the methodology saturates with a
/// handful of connections; 4 matches the reference clients' starting load.
const CONNECTIONS: usize = 4;

/// Upload POST body size per request (2 MB, matching the Cloudflare loop).
const UPLOAD_CHUNK_BYTES: usize = 2_000_000;

/// Aggregate sampling tick across the parallel connections.
const SAMPLE_INTERVAL: Duration = Duration::from_millis(500);

/// Default auto-mode durations (seconds): mirrors the Cloudflare split.
const AUTO_DOWNLOAD_SECS: u64 = 15;
const AUTO_UPLOAD_SECS: u64 = 10;

fn remaining_budget(deadline: Instant) -> Option<Duration> {
    let remaining = deadline.saturating_duration_since(Instant::now());
    (!remaining.is_zero()).then_some(remaining)
}

/// Run the Apple networkQuality capacity test.
pub async fn run<F>(config: &SpeedTestConfig, progress: F) -> ProviderResult
where
    F: Fn(Phase, f64) + Send + Sync,
{
    match tokio::time::timeout(provider_budget(config), run_inner(config, &progress)).await {
        Ok(Ok(result)) => result,
        Ok(Err(e)) => error_result(e),
        Err(_) => {
            error_result("Apple networkQuality exceeded its whole-provider deadline".to_string())
        }
    }
}

fn provider_budget(config: &SpeedTestConfig) -> Duration {
    let seconds = match config.duration {
        TestDuration::Seconds(seconds) => seconds.saturating_mul(3).saturating_add(40),
        TestDuration::Auto => AUTO_DOWNLOAD_SECS
            .saturating_mul(2)
            .saturating_add(AUTO_UPLOAD_SECS)
            .saturating_add(40),
    };
    Duration::from_secs(seconds)
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

    // ── Dense idle-latency engine (METHODOLOGY.md §4) ────────────────
    let latency_secs = match &config.duration {
        TestDuration::Seconds(s) => *s as f64,
        TestDuration::Auto => AUTO_DOWNLOAD_SECS as f64,
    };
    let n_probes = super::dense_probe_count(latency_secs);
    let mut rtts: Vec<f64> = Vec::with_capacity(n_probes as usize);
    for i in 0..n_probes {
        let start = Instant::now();
        if let Ok(resp) = client
            .get(cache_bust(&parsed.small_url))
            .timeout(Duration::from_secs(5))
            .send()
            .await
        {
            // Drain the (tiny) body so the next probe reuses the connection.
            let _ = resp.bytes().await;
            rtts.push(start.elapsed().as_secs_f64() * 1000.0);
        }
        if i + 1 < n_probes {
            tokio::time::sleep(super::DENSE_PROBE_INTERVAL).await;
        }
        // Discovery includes the dense latency phase; emit heartbeats so a
        // slow endpoint never looks like a frozen provider.
        progress(
            Phase::AnqDiscovery,
            0.5 + 0.5 * (f64::from(i + 1) / f64::from(n_probes)),
        );
    }
    // Discard the first 3 warm-up probes; keep all if ≤ 3.
    let measured: Vec<f64> = if rtts.len() > super::DENSE_PROBE_WARMUP {
        rtts[super::DENSE_PROBE_WARMUP..].to_vec()
    } else {
        rtts.clone()
    };
    let latency_stats = LatencyStats::from_rtts(&measured);
    let ping_ms = measured
        .iter()
        .copied()
        .min_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let jitter_ms = latency_stats.as_ref().map(|ls| ls.pdv);

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
        availability: ProviderAvailability::Ran,
        latency_stats,
        loaded_latency: None,
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

    let mut workers = tokio::task::JoinSet::new();
    for worker_id in 0..CONNECTIONS {
        let client = client.clone();
        let counter = counter.clone();
        match &spec {
            SaturateSpec::Download { url } => {
                let url = url.clone();
                workers.spawn(download_worker(client, counter, url, deadline, worker_id));
            }
            SaturateSpec::Upload { url } => {
                let url = url.clone();
                workers.spawn(upload_worker(client, counter, url, deadline, worker_id));
            }
        }
    }

    // Aggregate sampler.
    let mut samples: Vec<f64> = Vec::new();
    let mut last_total: u64 = 0;
    let mut last_at = Instant::now();
    let mut sampler = tokio::time::interval(SAMPLE_INTERVAL);
    sampler.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    sampler.reset();

    let phase_deadline = tokio::time::Instant::from_std(deadline);
    loop {
        tokio::select! {
            completed = workers.join_next(), if !workers.is_empty() => {
                if completed.is_none() || workers.is_empty() {
                    break;
                }
            }
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
            _ = tokio::time::sleep_until(phase_deadline) => break,
        }
    }
    // Explicitly abort and reap any request that did not observe the phase
    // deadline so it cannot consume bandwidth after the provider advances.
    workers.abort_all();
    while workers.join_next().await.is_some() {}

    (
        samples,
        counter.load(Ordering::Relaxed),
        start.elapsed().as_secs_f64(),
    )
}

async fn download_worker(
    client: Client,
    counter: Arc<AtomicU64>,
    url: String,
    deadline: Instant,
    worker_id: usize,
) {
    let worker = async {
        let mut failures = 0u32;
        while let Some(request_budget) = remaining_budget(deadline) {
            let Ok(resp) = client
                .get(&url)
                .timeout(request_budget.min(Duration::from_secs(30)))
                .send()
                .await
            else {
                failures = failures.saturating_add(1);
                sleep_retry_backoff(deadline, failures, worker_id).await;
                continue;
            };
            if !resp.status().is_success() {
                failures = failures.saturating_add(1);
                sleep_retry_backoff(deadline, failures, worker_id).await;
                continue;
            }

            // Stream the (8 GB+) object chunk by chunk; the outer deadline
            // cancels a peer that sends headers and then stalls the body.
            let mut resp = resp;
            let mut received = 0u64;
            while let Ok(Some(chunk)) = resp.chunk().await {
                counter.fetch_add(chunk.len() as u64, Ordering::Relaxed);
                received = received.saturating_add(chunk.len() as u64);
            }
            if received == 0 {
                failures = failures.saturating_add(1);
                sleep_retry_backoff(deadline, failures, worker_id).await;
            } else {
                failures = 0;
            }
        }
    };
    let _ = tokio::time::timeout_at(tokio::time::Instant::from_std(deadline), worker).await;
}

async fn upload_worker(
    client: Client,
    counter: Arc<AtomicU64>,
    url: String,
    deadline: Instant,
    worker_id: usize,
) {
    let worker = async {
        let payload = vec![0u8; UPLOAD_CHUNK_BYTES];
        let mut failures = 0u32;
        while let Some(request_budget) = remaining_budget(deadline) {
            match client
                .post(&url)
                .body(payload.clone())
                .timeout(request_budget.min(Duration::from_secs(30)))
                .send()
                .await
            {
                Ok(resp) if resp.status().is_success() => {
                    counter.fetch_add(UPLOAD_CHUNK_BYTES as u64, Ordering::Relaxed);
                    failures = 0;
                }
                _ => {
                    failures = failures.saturating_add(1);
                    sleep_retry_backoff(deadline, failures, worker_id).await;
                }
            }
        }
    };
    let _ = tokio::time::timeout_at(tokio::time::Instant::from_std(deadline), worker).await;
}

fn retry_backoff(failures: u32, worker_id: usize) -> Duration {
    let exponent = failures.saturating_sub(1).min(5);
    let base_ms = 25u64.saturating_mul(1u64 << exponent);
    // Deterministic per-worker jitter prevents four failed connections from
    // retrying in lockstep without adding a random-number dependency.
    let jitter_ms = ((u64::from(failures) * 37 + worker_id as u64 * 53) % 75) + 1;
    Duration::from_millis((base_ms + jitter_ms).min(1_000))
}

async fn sleep_retry_backoff(deadline: Instant, failures: u32, worker_id: usize) {
    let remaining = deadline.saturating_duration_since(Instant::now());
    if !remaining.is_zero() {
        tokio::time::sleep(retry_backoff(failures, worker_id).min(remaining)).await;
    }
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
        availability: ProviderAvailability::Failed,
        latency_stats: None,
        loaded_latency: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;
    use tokio::task::JoinHandle;

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

    #[test]
    fn retry_backoff_is_capped_and_jittered() {
        assert!(retry_backoff(2, 0) > retry_backoff(1, 0));
        assert_ne!(retry_backoff(3, 0), retry_backoff(3, 1));
        assert!(retry_backoff(100, 3) <= Duration::from_secs(1));
    }

    #[test]
    fn provider_deadline_scales_with_requested_duration() {
        let config = SpeedTestConfig {
            duration: TestDuration::Seconds(30),
            ..SpeedTestConfig::default()
        };
        assert_eq!(provider_budget(&config), Duration::from_secs(130));
    }

    async fn rejecting_server() -> (String, Arc<AtomicUsize>, JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let attempts = Arc::new(AtomicUsize::new(0));
        let server_attempts = attempts.clone();
        let task = tokio::spawn(async move {
            loop {
                let Ok((mut socket, _)) = listener.accept().await else {
                    break;
                };
                server_attempts.fetch_add(1, Ordering::SeqCst);
                tokio::spawn(async move {
                    let _ = socket
                        .write_all(
                            b"HTTP/1.1 503 Service Unavailable\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                        )
                        .await;
                    let _ = socket.shutdown().await;
                });
            }
        });
        (format!("http://{address}/large"), attempts, task)
    }

    async fn hanging_server() -> (String, Arc<AtomicUsize>, Arc<AtomicUsize>, JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let accepted = Arc::new(AtomicUsize::new(0));
        let closed = Arc::new(AtomicUsize::new(0));
        let server_accepted = accepted.clone();
        let server_closed = closed.clone();
        let task = tokio::spawn(async move {
            loop {
                let Ok((mut socket, _)) = listener.accept().await else {
                    break;
                };
                server_accepted.fetch_add(1, Ordering::SeqCst);
                let closed = server_closed.clone();
                tokio::spawn(async move {
                    // Consume the request but deliberately never send headers.
                    // Cancellation is observed as EOF when reqwest drops the
                    // in-flight connection.
                    let mut buffer = [0u8; 1024];
                    loop {
                        match socket.read(&mut buffer).await {
                            Ok(0) | Err(_) => {
                                closed.fetch_add(1, Ordering::SeqCst);
                                break;
                            }
                            Ok(_) => {}
                        }
                    }
                });
            }
        });
        (format!("http://{address}/large"), accepted, closed, task)
    }

    async fn wait_for_count(counter: &AtomicUsize, expected: usize) {
        tokio::time::timeout(Duration::from_secs(2), async {
            while counter.load(Ordering::SeqCst) < expected {
                tokio::task::yield_now().await;
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .expect("fault-injection server did not observe cancellation in time");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn transient_failures_retry_with_backoff_instead_of_spinning() {
        let (url, attempts, server) = rejecting_server().await;
        let client = Client::builder().no_proxy().build().unwrap();
        let (samples, bytes, _) = saturate(
            &client,
            SaturateSpec::Download { url },
            Duration::from_millis(350),
            |_| {},
        )
        .await;
        server.abort();
        let _ = server.await;

        let attempts = attempts.load(Ordering::SeqCst);
        assert!(attempts > CONNECTIONS, "each failed worker should retry");
        assert!(
            attempts < 40,
            "capped exponential backoff must prevent a retry storm: {attempts} attempts"
        );
        assert!(samples.is_empty());
        assert_eq!(bytes, 0);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn phase_deadline_cancels_stalled_requests_and_reaps_workers() {
        let (url, accepted, closed, server) = hanging_server().await;
        let client = Client::builder().no_proxy().build().unwrap();
        let started = Instant::now();
        let (samples, bytes, _) = saturate(
            &client,
            SaturateSpec::Download { url },
            Duration::from_millis(150),
            |_| {},
        )
        .await;

        assert!(started.elapsed() < Duration::from_secs(1));
        let accepted_count = accepted.load(Ordering::SeqCst);
        assert!(accepted_count > 0);
        wait_for_count(&closed, accepted_count).await;
        server.abort();
        let _ = server.await;
        assert!(samples.is_empty());
        assert_eq!(bytes, 0);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn caller_cancellation_drops_all_in_flight_connections() {
        let (url, accepted, closed, server) = hanging_server().await;
        let client = Client::builder().no_proxy().build().unwrap();
        let saturation = tokio::spawn(async move {
            saturate(
                &client,
                SaturateSpec::Download { url },
                Duration::from_secs(30),
                |_| {},
            )
            .await
        });

        wait_for_count(&accepted, CONNECTIONS).await;
        let accepted_count = accepted.load(Ordering::SeqCst);
        saturation.abort();
        assert!(saturation.await.unwrap_err().is_cancelled());
        wait_for_count(&closed, accepted_count).await;
        server.abort();
        let _ = server.await;
    }
}
