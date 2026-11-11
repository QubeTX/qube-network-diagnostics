use super::adaptive::adaptive_chunk_bytes;
use super::{
    statistics, BandwidthSamples, LatencyStats, LoadedLatency, Phase, ProviderAvailability,
    ProviderResult, ProviderSet, SpeedTestConfig, TestDuration,
};
use reqwest::Client;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const LATENCY_URL: &str = "https://speed.cloudflare.com/__down?bytes=0";
const DOWNLOAD_URL_BASE: &str = "https://speed.cloudflare.com/__down?bytes=";
const DOWNLOAD_MIN_BYTES: u64 = 1_000_000;
const DOWNLOAD_MAX_BYTES: u64 = 25_000_000;
const UPLOAD_URL: &str = "https://speed.cloudflare.com/__up";

/// Adaptive upload sizing (see `adaptive::adaptive_chunk_bytes`): each POST
/// targets ~2s at the previously measured throughput, keeping the sample rate
/// constant across link speeds.
const TARGET_REQUEST_SECS: f64 = 2.0;
const UPLOAD_MIN_BYTES: u64 = 256 * 1024;
const UPLOAD_MAX_BYTES: u64 = 16_000_000;

/// Minimum per-request timeout floor. A request started just before the phase
/// deadline still gets at least this long so a near-deadline request isn't
/// instantly aborted — the deadline-based `while` loop stops launching new ones.
const MIN_REQUEST_TIMEOUT: Duration = Duration::from_secs(1);

/// Per-probe timeout for the dense idle-latency engine.
const LATENCY_PROBE_TIMEOUT: Duration = Duration::from_secs(5);

/// Blackhole guard for the dense idle-latency engine. A server that completes
/// TCP/TLS but then drops every `HEAD` burns [`LATENCY_PROBE_TIMEOUT`] per
/// probe; unlike the transfer loops the dense engine has no phase deadline, so
/// without this a dead edge would stall the provider for `n_probes ×` timeout.
/// The loop bails after this many probes fail back-to-back — a working link
/// (even a high-RTT one) records successes and resets the counter, so this
/// never truncates a link that is actually responding.
const LATENCY_BLACKHOLE_LIMIT: u32 = 5;

/// Margin held below the orchestrator's FAST hard cap ([`super::FAST_HARD_CAP`])
/// for Cloudflare's internal FAST soft deadline. The dense-latency engine is
/// RTT-scaled and unbounded, so on high-RTT links the whole run (latency +
/// download + upload) could otherwise exceed the hard cap and be killed and
/// discarded by `run_provider_future`. Bounding the run a margin below the cap
/// lets it return the real data it has (§8 "reports what it has"); the margin
/// covers a final in-flight transfer overshooting its phase deadline by up to
/// [`MIN_REQUEST_TIMEOUT`] plus probe/task teardown.
const FAST_BUDGET_MARGIN: Duration = Duration::from_secs(3);

/// Loaded-latency probe cadence during saturation (METHODOLOGY.md §7: 200 ms
/// throttle, max 50 points per direction).
const LOADED_PROBE_INTERVAL: Duration = Duration::from_millis(200);
const MAX_LOADED_PROBES: usize = 50;
const LOADED_PROBE_TIMEOUT: Duration = Duration::from_secs(2);

/// EB-CS confidence level for FAST-mode early stopping (METHODOLOGY.md §8).
const EBCS_ALPHA: f64 = 0.05;

/// Time left until `deadline`, floored at `MIN_REQUEST_TIMEOUT`. Used as the
/// per-request `.timeout()` so a stalled transfer can't outlive the phase.
fn remaining_budget(deadline: Instant) -> Duration {
    deadline
        .saturating_duration_since(Instant::now())
        .max(MIN_REQUEST_TIMEOUT)
}

/// In FAST mode, clamp a phase deadline to the provider-wide soft deadline so
/// the whole run finishes under the orchestrator's hard cap and its completed
/// work is never discarded. In FULL mode (`fast_deadline == None`) the phase
/// keeps its own deadline unchanged.
fn clamp_deadline(deadline: Instant, fast_deadline: Option<Instant>) -> Instant {
    match fast_deadline {
        Some(fd) => deadline.min(fd),
        None => deadline,
    }
}

/// Cache-busting variant of a URL that already carries a query string.
fn cache_bust(url: &str) -> String {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    format!("{url}&_t={nanos}")
}

/// Spawn a best-effort loaded-latency probe loop that samples RTT every
/// [`LOADED_PROBE_INTERVAL`] into `acc` (capped at [`MAX_LOADED_PROBES`]) until
/// `stop` is set. Runs alongside a saturation phase (METHODOLOGY.md §7).
fn spawn_loaded_probe(
    client: Client,
    acc: Arc<Mutex<Vec<f64>>>,
    stop: Arc<AtomicBool>,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        while !stop.load(Ordering::Relaxed) {
            let start = Instant::now();
            if client
                .head(cache_bust(LATENCY_URL))
                .timeout(LOADED_PROBE_TIMEOUT)
                .send()
                .await
                .is_ok()
            {
                let rtt = start.elapsed().as_secs_f64() * 1000.0;
                if let Ok(mut v) = acc.lock() {
                    if v.len() < MAX_LOADED_PROBES {
                        v.push(rtt);
                    }
                }
            }
            tokio::time::sleep(LOADED_PROBE_INTERVAL).await;
        }
    })
}

/// Reclaim the loaded-latency samples once the probe task has been joined.
fn take_loaded(acc: Arc<Mutex<Vec<f64>>>) -> Vec<f64> {
    match Arc::try_unwrap(acc) {
        Ok(m) => m.into_inner().unwrap_or_default(),
        Err(shared) => shared.lock().map(|v| v.clone()).unwrap_or_default(),
    }
}

/// Run the Cloudflare speed test: dense idle latency, download, upload, with
/// loaded-latency probes during saturation (for delta-ms bufferbloat + RPM).
pub async fn run<F>(config: &SpeedTestConfig, progress: F) -> ProviderResult
where
    F: Fn(Phase, f64) + Send + Sync,
{
    let client = match Client::builder().timeout(Duration::from_secs(120)).build() {
        Ok(c) => c,
        Err(e) => return error_result(format!("Failed to build HTTP client: {e}")),
    };

    let fast = config.provider_set == ProviderSet::Fast;

    // Provider-wide FAST soft deadline: cap the whole run (latency + download +
    // upload) a margin below the orchestrator's 25 s hard cap so this provider
    // always returns the real data it has gathered rather than being killed by
    // the outer timeout and discarded (METHODOLOGY.md §8 "reports what it has").
    let run_start = Instant::now();
    let fast_deadline =
        fast.then(|| run_start + super::FAST_HARD_CAP.saturating_sub(FAST_BUDGET_MARGIN));

    // ── Duration per direction ──────────────────────────────────────
    let (dl_secs, ul_secs) = match &config.duration {
        TestDuration::Seconds(s) => (*s, *s),
        TestDuration::Auto => (15, 15),
    };

    // ── Dense idle-latency engine (METHODOLOGY.md §4) ────────────────
    progress(Phase::CfLatency, 0.0);

    let n_probes = super::dense_probe_count(dl_secs as f64);
    let mut rtts: Vec<f64> = Vec::with_capacity(n_probes as usize);
    let mut failures: u32 = 0;
    let mut probes_done: u32 = 0;
    let mut consecutive_failures: u32 = 0;

    for i in 0..n_probes {
        // FAST provider budget: yield the RTT-scaled dense engine to the overall
        // soft deadline so the provider returns before the outer hard cap can
        // discard its completed work (METHODOLOGY.md §8).
        if let Some(fd) = fast_deadline {
            if Instant::now() >= fd {
                break;
            }
        }
        progress(Phase::CfLatency, i as f64 / n_probes as f64);
        probes_done += 1;
        // Bound a single probe by the FAST deadline too, so a stalled HEAD near
        // the deadline can't overshoot the hard cap.
        let probe_timeout = match fast_deadline {
            Some(fd) => LATENCY_PROBE_TIMEOUT.min(fd.saturating_duration_since(Instant::now())),
            None => LATENCY_PROBE_TIMEOUT,
        };
        let start = Instant::now();
        match client
            .head(cache_bust(LATENCY_URL))
            .timeout(probe_timeout)
            .send()
            .await
        {
            Ok(_) => {
                rtts.push(start.elapsed().as_secs_f64() * 1000.0);
                consecutive_failures = 0;
            }
            Err(_) => {
                failures += 1;
                consecutive_failures += 1;
                if consecutive_failures >= LATENCY_BLACKHOLE_LIMIT {
                    break;
                }
            }
        }
        if i + 1 < n_probes {
            tokio::time::sleep(super::DENSE_PROBE_INTERVAL).await;
        }
    }

    progress(Phase::CfLatency, 1.0);

    // Discard the first 3 warm-up probes (DNS/TCP/TLS); keep all if ≤ 3.
    let measured: Vec<f64> = if rtts.len() > super::DENSE_PROBE_WARMUP {
        rtts[super::DENSE_PROBE_WARMUP..].to_vec()
    } else {
        rtts.clone()
    };
    let latency_stats = LatencyStats::from_rtts(&measured);
    // Min-RTT headline; jitter = PDV (canonical) from the dense block.
    let ping_ms = measured
        .iter()
        .copied()
        .min_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let jitter_ms = latency_stats.as_ref().map(|ls| ls.pdv);
    let min_rtt_gate = ping_ms.unwrap_or(f64::INFINITY);

    // Denominator = probes actually attempted (equals `n_probes` on a full run;
    // fewer if the FAST deadline or the blackhole guard cut the loop short).
    let packet_loss_pct = if probes_done > 0 {
        Some(failures as f64 / probes_done as f64 * 100.0)
    } else {
        None
    };

    // ── Download phase (with concurrent loaded-latency probes) ───────
    progress(Phase::CfDownload, 0.0);

    let loaded_dl = Arc::new(Mutex::new(Vec::<f64>::new()));
    let dl_probe_stop = Arc::new(AtomicBool::new(false));
    let dl_probe_handle =
        spawn_loaded_probe(client.clone(), loaded_dl.clone(), dl_probe_stop.clone());

    let dl_deadline = clamp_deadline(Instant::now() + Duration::from_secs(dl_secs), fast_deadline);
    let mut dl_bytes: u64 = 0;
    let dl_start = Instant::now();
    let mut dl_mbps_samples: Vec<f64> = Vec::new();
    // Adaptive download sizing (mirrors the upload loop): start at 1 MB and
    // grow toward ~2 s per request. Fixed 10 MB requests both under-sample slow
    // links and trip speed.cloudflare.com's volumetric per-IP rate limit faster.
    let mut dl_chunk_bytes: u64 = DOWNLOAD_MIN_BYTES;
    let mut rate_limited_retry_s: Option<u64> = None;

    while Instant::now() < dl_deadline {
        let req_start = Instant::now();
        let url = format!("{DOWNLOAD_URL_BASE}{dl_chunk_bytes}");
        match client
            .get(&url)
            .timeout(remaining_budget(dl_deadline))
            .send()
            .await
        {
            Ok(resp) if resp.status().as_u16() == 429 => {
                // Volumetric rate limit on large payloads. Requests will keep
                // failing for minutes — spinning here only deepens the penalty.
                rate_limited_retry_s = Some(
                    resp.headers()
                        .get("retry-after")
                        .and_then(|v| v.to_str().ok())
                        .and_then(|s| s.parse::<u64>().ok())
                        .unwrap_or(0),
                );
                break;
            }
            Ok(resp) if resp.status().is_success() => {
                if let Ok(body) = resp.bytes().await {
                    let req_bytes = body.len() as u64;
                    let req_duration = req_start.elapsed().as_secs_f64();
                    dl_bytes += req_bytes;
                    if req_duration > 0.0 {
                        let mbps = (req_bytes as f64 * 8.0) / (req_duration * 1_000_000.0);
                        dl_mbps_samples.push(mbps);
                        dl_chunk_bytes = adaptive_chunk_bytes(
                            mbps,
                            TARGET_REQUEST_SECS,
                            DOWNLOAD_MIN_BYTES,
                            DOWNLOAD_MAX_BYTES,
                        );
                    }
                    let elapsed = dl_start.elapsed().as_secs_f64();
                    let frac = (elapsed / dl_secs as f64).min(1.0);
                    progress(Phase::CfDownload, frac);
                }
            }
            Err(_) => {
                dl_chunk_bytes = DOWNLOAD_MIN_BYTES;
            }
            _ => {}
        }

        // FAST-mode anytime-valid early stop (RTT-gated inside the CS).
        if fast
            && statistics::empirical_bernstein_cs(&dl_mbps_samples, EBCS_ALPHA, min_rtt_gate).stop
        {
            break;
        }
    }

    dl_probe_stop.store(true, Ordering::Relaxed);
    let _ = dl_probe_handle.await;
    let loaded_download = take_loaded(loaded_dl);

    let dl_elapsed = dl_start.elapsed().as_secs_f64();
    progress(Phase::CfDownload, 1.0);

    let download_mbps = if dl_mbps_samples.is_empty() {
        None
    } else {
        Some(statistics::accurate_bandwidth(&dl_mbps_samples))
    };

    // ── Upload phase (with concurrent loaded-latency probes) ─────────
    progress(Phase::CfUpload, 0.0);

    let loaded_ul = Arc::new(Mutex::new(Vec::<f64>::new()));
    let ul_probe_stop = Arc::new(AtomicBool::new(false));
    let ul_probe_handle =
        spawn_loaded_probe(client.clone(), loaded_ul.clone(), ul_probe_stop.clone());

    let ul_deadline = clamp_deadline(Instant::now() + Duration::from_secs(ul_secs), fast_deadline);
    let mut ul_bytes: u64 = 0;
    let ul_start = Instant::now();
    let mut ul_mbps_samples: Vec<f64> = Vec::new();
    // Adaptive payload sizing; the buffer is only rebuilt when the size changes.
    let mut ul_chunk_bytes: u64 = UPLOAD_MIN_BYTES.max(1_000_000);
    let mut upload_payload = vec![0u8; ul_chunk_bytes as usize];

    while Instant::now() < ul_deadline {
        if upload_payload.len() as u64 != ul_chunk_bytes {
            upload_payload = vec![0u8; ul_chunk_bytes as usize];
        }
        let req_bytes = upload_payload.len() as u64;
        let req_start = Instant::now();
        match client
            .post(UPLOAD_URL)
            .body(upload_payload.clone())
            .timeout(remaining_budget(ul_deadline))
            .send()
            .await
        {
            Ok(resp) if resp.status().as_u16() == 429 => {
                rate_limited_retry_s = Some(
                    resp.headers()
                        .get("retry-after")
                        .and_then(|v| v.to_str().ok())
                        .and_then(|s| s.parse::<u64>().ok())
                        .unwrap_or(0),
                );
                break;
            }
            Ok(resp) if resp.status().is_success() => {
                let req_duration = req_start.elapsed().as_secs_f64();
                ul_bytes += req_bytes;
                if req_duration > 0.0 {
                    let mbps = (req_bytes as f64 * 8.0) / (req_duration * 1_000_000.0);
                    ul_mbps_samples.push(mbps);
                    ul_chunk_bytes = adaptive_chunk_bytes(
                        mbps,
                        TARGET_REQUEST_SECS,
                        UPLOAD_MIN_BYTES,
                        UPLOAD_MAX_BYTES,
                    );
                }
                let elapsed = ul_start.elapsed().as_secs_f64();
                let frac = (elapsed / ul_secs as f64).min(1.0);
                progress(Phase::CfUpload, frac);
            }
            Err(_) => {
                ul_chunk_bytes = UPLOAD_MIN_BYTES;
            }
            _ => {}
        }

        if fast
            && statistics::empirical_bernstein_cs(&ul_mbps_samples, EBCS_ALPHA, min_rtt_gate).stop
        {
            break;
        }
    }

    ul_probe_stop.store(true, Ordering::Relaxed);
    let _ = ul_probe_handle.await;
    let loaded_upload = take_loaded(loaded_ul);

    let ul_elapsed = ul_start.elapsed().as_secs_f64();
    progress(Phase::CfUpload, 1.0);

    let upload_mbps = if ul_mbps_samples.is_empty() {
        None
    } else {
        Some(statistics::accurate_upload_bandwidth(&ul_mbps_samples))
    };

    let loaded_latency = if loaded_download.is_empty() && loaded_upload.is_empty() {
        None
    } else {
        Some(LoadedLatency {
            download: loaded_download,
            upload: loaded_upload,
        })
    };

    // Honest failure: latency probing reached the server but neither direction
    // produced a single usable transfer sample. A 429 is disclosed even on
    // partial success so the excluded direction is explainable.
    let error = if dl_mbps_samples.is_empty() && ul_mbps_samples.is_empty() {
        match rate_limited_retry_s {
            Some(retry) if retry > 0 => Some(format!(
                "rate-limited by speed.cloudflare.com (HTTP 429, retry in ~{}m)",
                retry.div_ceil(60)
            )),
            Some(_) => Some("rate-limited by speed.cloudflare.com (HTTP 429)".to_string()),
            None => Some("no successful transfers".to_string()),
        }
    } else {
        match rate_limited_retry_s {
            Some(retry) if retry > 0 => Some(format!(
                "partial: rate-limited by speed.cloudflare.com (HTTP 429, retry in ~{}m)",
                retry.div_ceil(60)
            )),
            Some(_) => Some("partial: rate-limited by speed.cloudflare.com (HTTP 429)".to_string()),
            None => None,
        }
    };
    // Failed only when NOTHING transferred — a partial 429 (one direction
    // measured, the other throttled) still ran and its data still merges.
    let availability = if dl_mbps_samples.is_empty() && ul_mbps_samples.is_empty() {
        ProviderAvailability::Failed
    } else {
        ProviderAvailability::Ran
    };

    ProviderResult {
        provider: "Cloudflare".to_string(),
        server: "speed.cloudflare.com".to_string(),
        location: None,
        ping_ms,
        jitter_ms,
        download_mbps,
        upload_mbps,
        download_bytes: dl_bytes,
        upload_bytes: ul_bytes,
        download_duration_s: dl_elapsed,
        upload_duration_s: ul_elapsed,
        packet_loss_pct,
        error,
        bandwidth_samples: Some(BandwidthSamples {
            download: dl_mbps_samples,
            upload: ul_mbps_samples,
        }),
        availability,
        latency_stats,
        loaded_latency,
    }
}

/// Build an error ProviderResult with zeroed metrics.
fn error_result(msg: String) -> ProviderResult {
    ProviderResult {
        provider: "Cloudflare".to_string(),
        server: "speed.cloudflare.com".to_string(),
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
