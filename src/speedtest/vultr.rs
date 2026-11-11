//! Vultr speed test provider (download-only, SpeedQX Methodology v4, §3).
//!
//! Ookla-style multi-PoP server selection: races a 2× `Range: bytes=0-0` probe
//! against 8 candidate PoPs concurrently (first probe per PoP discarded as
//! connection warm-up, second kept) and picks the PoP with the lowest kept
//! RTT. Download throughput is then measured via adaptively-sized ranged GETs
//! against the winning PoP's 100 MiB test file, streamed through the response
//! body (via [`reqwest::Response::chunk`], no extra Cargo feature needed) and
//! tick-sampled every 250ms on a clock that is independent of request/chunk
//! boundaries — the same general technique used by `cachefly.rs`, implemented
//! independently here (no shared code, matching the production TypeScript
//! providers' own convention: `vultr-provider.ts` and `cachefly-provider.ts`
//! each re-implement this ticker rather than sharing it). Latency is a
//! dedicated series of 10 ranged 1-byte probes to the chosen PoP (first 2
//! discarded as warm-up, headline = min of the remaining 8).
//!
//! Upload is not supported (no `upload_mbps`) — Vultr only publishes a public
//! download test file per PoP. Packet loss is not measured (no TURN/UDP path
//! here). See METHODOLOGY.md §3 (capability prior 0.95) and §5 (per-provider
//! pipeline) for how this slots into the cross-provider merge.
//!
//! ── What this provider deliberately does NOT do ─────────────────────────
//! Same as `cachefly.rs`: no bootstrap variance / BCa interval here — the v4
//! circular block bootstrap draws from a single PCG32 stream threaded across
//! ALL providers in registry order, which is the cross-provider orchestrator's
//! job (`mod.rs::aggregate`). This provider exposes raw, time-ordered ~250ms
//! Mbps samples (`bandwidth_samples`) for that purpose, plus its own
//! same-pipeline-minus-bootstrap point estimate as `download_mbps`.

use super::{
    statistics, BandwidthSamples, Phase, ProviderAvailability, ProviderResult, SpeedTestConfig,
    TestDuration,
};
use reqwest::Client;
use std::time::{Duration, Instant};

// ── Candidate PoPs (per the provider registry; ACAO: *, Range-supported) ───
const CANDIDATE_POPS: [&str; 8] = [
    "nj-us",
    "lax-ca-us",
    "fra-de",
    "sgp",
    "syd-au",
    "tyo-jp",
    "ams-nl",
    "lon-gb",
];

fn pop_display_name(pop: &str) -> &'static str {
    match pop {
        "nj-us" => "New Jersey, US",
        "lax-ca-us" => "Los Angeles, US",
        "fra-de" => "Frankfurt, DE",
        "sgp" => "Singapore",
        "syd-au" => "Sydney, AU",
        "tyo-jp" => "Tokyo, JP",
        "ams-nl" => "Amsterdam, NL",
        "lon-gb" => "London, GB",
        _ => "Unknown",
    }
}

fn pop_url(pop: &str) -> String {
    format!("https://{pop}-ping.vultr.com/vultr.com.100MB.bin")
}

/// Verified via `Content-Range: bytes 0-0/104857600`.
const FILE_SIZE_BYTES: u64 = 104_857_600;

// ── Download tuning ─────────────────────────────────────────────────────
/// Throughput tick cadence — independent of request/chunk boundaries.
const SAMPLE_TICK_MS: f64 = 250.0;
/// First ranged request, before any rate estimate exists.
const INITIAL_CHUNK_BYTES: u64 = 1_000_000;
/// Floor so slow links don't collapse into request-overhead-dominated chunks.
const MIN_CHUNK_BYTES: u64 = 262_144;
/// Ceiling — comfortably under the 100 MiB file even on very fast links.
const MAX_CHUNK_BYTES: u64 = 50_000_000;
/// "~2s of transfer at the last measured rate" per METHODOLOGY.md §5 step 1.
const TARGET_CHUNK_SECONDS: f64 = 2.0;
/// Per-chunk safety timeout — guards against a single stalled request eating
/// the whole test (matches `cachefly.rs`'s identical constant).
const CHUNK_TIMEOUT: Duration = Duration::from_secs(15);
const CHUNK_RETRY_BACKOFF: Duration = Duration::from_millis(250);

const DEFAULT_TEST_SECONDS: u64 = 10;

const LATENCY_PROBE_COUNT: u32 = 10;
const LATENCY_DISCARD_COUNT: usize = 2;
/// Per-probe safety timeout for the latency-ladder phase (the TS reference
/// applies no explicit override here, relying on the outer client timeout —
/// this is a deliberate, defense-in-depth addition, harmless since the PoP is
/// already confirmed reachable during selection).
const LATENCY_PROBE_TIMEOUT: Duration = Duration::from_secs(5);

/// Per-probe safety timeout for PoP selection. A SYN-blackholed / firewalled
/// PoP never rejects on its own — the request just hangs — so without this a
/// single unreachable candidate would stall the whole run. Bounds each ranged
/// RTT probe independently; `select_pop`'s outer `tokio::time::timeout` is a
/// second, coarser bound around the whole 2-probe sequence per PoP (defense
/// in depth — a dead PoP must NOT stall selection).
const POP_PROBE_TIMEOUT: Duration = Duration::from_secs(5);

/// Run the Vultr speed test: PoP selection, latency, then the adaptive ranged download.
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

    let test_secs = match &config.duration {
        TestDuration::Seconds(s) => *s,
        TestDuration::Auto => DEFAULT_TEST_SECONDS,
    };

    // ── PoP selection: 2× ranged probe per candidate, concurrently, min-RTT ──
    progress(Phase::VultrDiscovery, 0.0);
    let selection = select_pop(&client).await;
    let Some(selection) = selection else {
        return Err("Vultr: no reachable candidate PoP — all probes failed".to_string());
    };
    progress(Phase::VultrDiscovery, 1.0);

    let display_name = pop_display_name(selection.pop);
    let server_name = format!("Vultr ({display_name})");
    let file_url = pop_url(selection.pop);

    // ── Latency: 10 ranged 1-byte probes to the chosen PoP, discard first 2 ──
    progress(Phase::VultrLatency, 0.0);
    let (ping_ms, latency_samples) = measure_latency_to_pop(&client, &file_url).await;
    let jitter_ms =
        (latency_samples.len() >= 2).then(|| statistics::jitter_rfc3550(&latency_samples));
    progress(Phase::VultrLatency, 1.0);

    // ── Download: adaptive-sized ranged GETs, 250ms streamed sampling ───────
    progress(Phase::VultrDownload, 0.0);
    let phase_start = Instant::now();
    let test_duration = Duration::from_secs(test_secs);
    let test_secs_f = test_secs as f64;
    let (raw_samples, total_download_bytes) =
        run_ranged_download(&client, &file_url, test_duration, |_mbps| {
            let elapsed = phase_start.elapsed().as_secs_f64();
            progress(Phase::VultrDownload, (elapsed / test_secs_f).min(1.0));
        })
        .await;
    let download_duration_s = phase_start.elapsed().as_secs_f64();

    // ── Point estimate (METHODOLOGY.md §5 steps 2/3/5/6, minus bootstrap — see
    // the module doc comment) ───────────────────────────────────────────────
    let after_plateau: Vec<f64> = if raw_samples.is_empty() {
        Vec::new()
    } else {
        raw_samples[statistics::plateau_start(&raw_samples).min(raw_samples.len())..].to_vec()
    };
    let cleaned = statistics::filter_outliers_iqr(&after_plateau, 1.5);
    let basis = if cleaned.is_empty() {
        after_plateau
    } else {
        cleaned
    };
    let download_speed = if basis.is_empty() {
        0.0
    } else {
        statistics::modified_trimean(&basis)
    };

    // Hodges-Lehmann cross-check (METHODOLOGY.md §5 step 6) — diagnostic-only,
    // never used to adjust `download_speed` (same schema gap noted in
    // `cachefly.rs`: `ProviderResult` has no field yet to surface this).
    let hl = if basis.is_empty() {
        0.0
    } else {
        statistics::hodges_lehmann(&basis)
    };
    let _unstable = download_speed > 0.0 && (hl - download_speed).abs() / download_speed > 0.15;

    progress(Phase::VultrDownload, 1.0);

    Ok(ProviderResult {
        provider: "Vultr".to_string(),
        server: server_name,
        location: Some(selection.pop.to_string()),
        ping_ms,
        jitter_ms,
        download_mbps: if raw_samples.is_empty() {
            None
        } else {
            Some(download_speed)
        },
        // Vultr never runs an upload phase (only a public download test file is
        // published per PoP). `None` is an ABSENT measurement, not a measured
        // zero — see the identical note in `cachefly.rs`.
        upload_mbps: None,
        download_bytes: total_download_bytes,
        upload_bytes: 0,
        download_duration_s,
        upload_duration_s: 0.0,
        packet_loss_pct: None,
        error: None,
        bandwidth_samples: Some(BandwidthSamples {
            download: raw_samples,
            upload: Vec::new(),
        }),
        availability: ProviderAvailability::Ran,
        latency_stats: None,
        loaded_latency: None,
    })
}

// ── PoP selection ───────────────────────────────────────────────────────

struct PopSelection {
    pop: &'static str,
    rtt: f64,
}

/// Single ranged `bytes=0-0` RTT probe. Wall-clock only: unlike a browser,
/// reqwest exposes no `PerformanceResourceTiming`-style TTFB breakdown, and
/// (per the TS reference's own note) Vultr's PoP hosts don't send
/// `Timing-Allow-Origin` anyway, so in practice only the wall-clock fallback
/// ever mattered there either.
async fn measure_ranged_rtt(
    client: &Client,
    url: &str,
    timeout: Duration,
) -> Result<f64, reqwest::Error> {
    let start = Instant::now();
    let resp = client
        .get(cache_bust_url(url))
        .header("Range", "bytes=0-0")
        .timeout(timeout)
        .send()
        .await?;
    let _ = resp.bytes().await; // drain the 1-byte body so the connection releases promptly
    Ok(start.elapsed().as_secs_f64() * 1000.0)
}

/// Probe one PoP: a discarded warm-up probe followed by the kept probe, both
/// bounded by [`POP_PROBE_TIMEOUT`] individually AND by an outer
/// `tokio::time::timeout` around the whole sequence — a blackholed PoP can
/// never stall `select_pop`'s `join_all`.
async fn probe_pop(client: &Client, pop: &'static str) -> Option<PopSelection> {
    let url = pop_url(pop);
    let outcome = tokio::time::timeout(POP_PROBE_TIMEOUT * 2, async {
        measure_ranged_rtt(client, &url, POP_PROBE_TIMEOUT)
            .await
            .ok()?; // warm-up — discarded
        measure_ranged_rtt(client, &url, POP_PROBE_TIMEOUT)
            .await
            .ok()
    })
    .await;
    match outcome {
        Ok(Some(rtt)) => Some(PopSelection { pop, rtt }),
        _ => None,
    }
}

/// Races the 2×-probe sequence across all candidate PoPs concurrently (via
/// `join_all`, each future individually timeout-bounded) and picks the
/// min-RTT reachable PoP. A PoP that fails or times out is simply absent from
/// the reachable set rather than sinking the whole selection.
async fn select_pop(client: &Client) -> Option<PopSelection> {
    let futures = CANDIDATE_POPS.iter().map(|&pop| probe_pop(client, pop));
    let results = futures_util::future::join_all(futures).await;
    let mut reachable: Vec<PopSelection> = results.into_iter().flatten().collect();
    if reachable.is_empty() {
        return None;
    }
    reachable.sort_by(|a, b| {
        a.rtt
            .partial_cmp(&b.rtt)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    Some(reachable.remove(0))
}

// ── Latency phase (10 probes, discard first 2, min) ────────────────────

async fn measure_latency_to_pop(client: &Client, url: &str) -> (Option<f64>, Vec<f64>) {
    let mut all: Vec<f64> = Vec::with_capacity(LATENCY_PROBE_COUNT as usize);
    for _ in 0..LATENCY_PROBE_COUNT {
        if let Ok(rtt) = measure_ranged_rtt(client, url, LATENCY_PROBE_TIMEOUT).await {
            all.push(rtt);
        }
    }
    // Only discard when there's enough to discard AND still have something
    // left; otherwise use whatever came back rather than collapsing to zero
    // samples (mirrors `vultr-provider.ts` exactly).
    let kept: Vec<f64> = if all.len() > LATENCY_DISCARD_COUNT {
        all[LATENCY_DISCARD_COUNT..].to_vec()
    } else {
        all
    };
    let ping = kept
        .iter()
        .copied()
        .min_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    (ping, kept)
}

// ── Download phase (adaptive ranged GETs, 250ms streamed sampling) ─────

/// Cache-busting query string (see `cachefly.rs`'s identical helper —
/// duplicated rather than shared, matching the TS reference providers' own
/// no-shared-code convention for this technique).
fn cache_bust_url(url: &str) -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    format!("{url}?_t={}&_r={}", now.as_millis(), now.as_nanos())
}

fn bytes_to_mbps(bytes: f64, elapsed_ms: f64) -> f64 {
    if elapsed_ms <= 0.0 || bytes <= 0.0 {
        return 0.0;
    }
    (bytes * 8.0) / (elapsed_ms / 1000.0) / 1_000_000.0
}

/// A 250ms throughput ticker whose clock runs continuously across chunk
/// boundaries (mirrors `cachefly.rs`'s `DownloadTicker`, re-implemented here
/// independently per the TS reference's own convention).
struct DownloadTicker {
    interval_start: Instant,
    interval_bytes: u64,
}

impl DownloadTicker {
    fn new(start: Instant) -> Self {
        Self {
            interval_start: start,
            interval_bytes: 0,
        }
    }

    fn add_bytes(&mut self, n: u64) {
        self.interval_bytes += n;
    }

    fn flush(
        &mut self,
        now: Instant,
        force: bool,
        raw_samples: &mut Vec<f64>,
        on_tick: &mut dyn FnMut(f64),
    ) {
        let elapsed_ms = now
            .saturating_duration_since(self.interval_start)
            .as_secs_f64()
            * 1000.0;
        if !force && elapsed_ms < SAMPLE_TICK_MS {
            return;
        }
        if elapsed_ms <= 0.0 {
            return;
        }
        if self.interval_bytes > 0 {
            let mbps = bytes_to_mbps(self.interval_bytes as f64, elapsed_ms);
            raw_samples.push(mbps);
            on_tick(mbps);
        }
        self.interval_bytes = 0;
        self.interval_start = now;
    }
}

/// Drain one ranged response into the continuous ticker, also tracking this
/// request's own byte count (for the next adaptive chunk-size calculation).
/// Stops draining early if `deadline` passes mid-stream (mirrors the TS
/// reference's `reader.cancel()` on deadline — here, simply dropping the
/// response lets reqwest release the connection).
async fn drain_ranged_chunk(
    mut resp: reqwest::Response,
    ticker: &mut DownloadTicker,
    raw_samples: &mut Vec<f64>,
    deadline: Instant,
    on_tick: &mut dyn FnMut(f64),
) -> (u64, f64) {
    let chunk_start = Instant::now();
    let mut chunk_bytes: u64 = 0;
    loop {
        match resp.chunk().await {
            Ok(Some(bytes)) => {
                let n = bytes.len() as u64;
                if n > 0 {
                    chunk_bytes += n;
                    ticker.add_bytes(n);
                }
                let now = Instant::now();
                ticker.flush(now, false, raw_samples, on_tick);
                if now >= deadline {
                    break;
                }
            }
            Ok(None) => break,
            Err(_) => break,
        }
    }
    (chunk_bytes, chunk_start.elapsed().as_secs_f64())
}

/// Adaptive-sized ranged GET loop against `url`'s 100 MiB file, walking the
/// byte offset forward (wrapping to 0 at EOF) each request, ticked every
/// 250ms. Returns the raw, time-ordered Mbps sample series plus the total
/// bytes transferred across the whole phase.
async fn run_ranged_download(
    client: &Client,
    url: &str,
    test_duration: Duration,
    mut on_tick: impl FnMut(f64),
) -> (Vec<f64>, u64) {
    let mut raw_samples = Vec::new();
    let start_time = Instant::now();
    let deadline = start_time + test_duration;
    let mut ticker = DownloadTicker::new(start_time);

    let mut offset: u64 = 0;
    let mut next_chunk_bytes: u64 = INITIAL_CHUNK_BYTES;
    let mut total_bytes: u64 = 0;

    while Instant::now() < deadline {
        let size = next_chunk_bytes.clamp(MIN_CHUNK_BYTES, MAX_CHUNK_BYTES);
        let range_start = offset;
        let range_end = (range_start + size).min(FILE_SIZE_BYTES).saturating_sub(1);

        let sent = client
            .get(cache_bust_url(url))
            .header("Range", format!("bytes={range_start}-{range_end}"))
            .timeout(CHUNK_TIMEOUT)
            .send()
            .await;

        let resp = match sent {
            Ok(r) if r.status().is_success() => r,
            Ok(_) => {
                // Unexpected status — restart from offset 0.
                offset = 0;
                continue;
            }
            Err(_) => {
                tokio::time::sleep(CHUNK_RETRY_BACKOFF).await;
                continue;
            }
        };

        let (chunk_bytes, chunk_secs) =
            drain_ranged_chunk(resp, &mut ticker, &mut raw_samples, deadline, &mut on_tick).await;
        total_bytes += chunk_bytes;

        if chunk_secs > 0.0 && chunk_bytes > 0 {
            let rate_bps = (chunk_bytes as f64 * 8.0) / chunk_secs;
            next_chunk_bytes = ((rate_bps * TARGET_CHUNK_SECONDS) / 8.0).round() as u64;
        }

        offset = if range_end + 1 >= FILE_SIZE_BYTES {
            0
        } else {
            range_end + 1
        };
    }

    ticker.flush(Instant::now(), true, &mut raw_samples, &mut on_tick); // trailing partial bucket
    (raw_samples, total_bytes)
}

/// Build an error `ProviderResult` with zeroed metrics.
fn error_result(msg: String) -> ProviderResult {
    ProviderResult {
        provider: "Vultr".to_string(),
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

    #[test]
    fn pop_display_names_cover_every_candidate() {
        for pop in CANDIDATE_POPS {
            assert_ne!(
                pop_display_name(pop),
                "Unknown",
                "missing display name for {pop}"
            );
        }
    }

    #[test]
    fn pop_url_shape() {
        assert_eq!(
            pop_url("nj-us"),
            "https://nj-us-ping.vultr.com/vultr.com.100MB.bin"
        );
    }

    #[test]
    fn bytes_to_mbps_matches_hand_computed() {
        assert!((bytes_to_mbps(10_000_000.0, 1000.0) - 80.0).abs() < 1e-9);
    }

    #[test]
    fn ticker_flushes_only_after_real_interval() {
        let start = Instant::now();
        let mut ticker = DownloadTicker::new(start);
        let mut samples = Vec::new();
        ticker.add_bytes(1_000_000);
        ticker.flush(start, false, &mut samples, &mut |_| {});
        assert!(samples.is_empty());

        let later = start + Duration::from_millis(10);
        ticker.flush(later, true, &mut samples, &mut |_| {});
        assert_eq!(samples.len(), 1);
    }

    #[test]
    fn error_result_has_no_measurements() {
        let r = error_result("boom".to_string());
        assert_eq!(r.provider, "Vultr");
        assert!(r.error.is_some());
        assert!(r.download_mbps.is_none());
        assert!(r.upload_mbps.is_none());
    }
}
