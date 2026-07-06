//! CacheFly speed test provider — HTTP download-only, HTTP range-based
//! (METHODOLOGY.md §3, capability prior 0.95).
//!
//! Public CDN test files at `cachefly.cachefly.net/{1,10,100}mb.test` (`ACAO: *`
//! and Range-request support live-verified 2026-07-06 per the TypeScript
//! reference implementation). There is no upload endpoint at all — CacheFly is
//! structurally download-only per the spec's own provider table.
//!
//! Progressive ladder (adaptive sizing via [`adaptive_chunk_bytes`], mirroring
//! `cloudflare.rs`'s upload sizing and CacheFly's own production TypeScript
//! provider, `cachefly-provider.ts`):
//!   1. `1mb.test` × 2 — fixed warm-up, timed as whole requests only (no
//!      interval sampling), discarded from the raw sample set; used solely to
//!      seed the first rate estimate for the mid tier.
//!   2. `10mb.test` × N — at least [`MIN_MID_TIER_REQUESTS`] full-file requests
//!      for a stable reading, then escalate once a full 10MB transfer
//!      undershoots the ~2s target at the last measured rate.
//!   3. `100mb.test` × N — `Range: bytes=0-{N-1}` (always from byte 0, per the
//!      TS reference — not a walking offset like `vultr.rs`), sized to ~2s at
//!      the last measured rate, repeated until the download budget is spent.
//!
//! Mid + large tier bytes are streamed through the response body (via
//! [`reqwest::Response::chunk`], which needs no extra Cargo feature) into a
//! SINGLE continuous 250ms ticker that spans request (and tier) boundaries —
//! the same technique `vultr.rs` uses, implemented independently here (no
//! shared code, matching the TS reference providers' own convention) so a
//! request boundary never manufactures a spurious tiny tail sample.
//!
//! Latency is a dedicated 10-probe `HEAD` + `Range: bytes=0-0` ladder against
//! `1mb.test` (first 2 discarded as TCP/TLS warm-up, headline = min RTT of the
//! remaining 8) — CacheFly's own edge path, feeding the cross-source min-RTT
//! headline (METHODOLOGY.md §4) alongside the shared Cloudflare latency engine
//! and every other provider's own ping.
//!
//! ── What this provider deliberately does NOT do ─────────────────────────
//! It does not compute its own bootstrap variance / BCa interval. The v4
//! circular block bootstrap (METHODOLOGY.md §5 step 7, [`statistics::circular_block_bootstrap`])
//! draws from a single PCG32 stream threaded across ALL providers in registry
//! order (cloudflare, ndt7, msak, librespeed, fastcom, cachefly, vultr, applenq
//! — see the v4 statistical notes), so that responsibility belongs to the
//! cross-provider orchestrator (`mod.rs::aggregate`), not to this file. This
//! provider exposes raw, time-ordered ~250ms Mbps samples (`bandwidth_samples`)
//! for that purpose, plus a same-pipeline-minus-bootstrap point estimate
//! (plateau discard → IQR k=1.5 → modified trimean, with a Hodges-Lehmann
//! instability cross-check) as its own reported `download_mbps` — matching
//! every other v4 provider file.

use super::adaptive::adaptive_chunk_bytes;
use super::{
    statistics, BandwidthSamples, Phase, ProviderAvailability, ProviderResult, SpeedTestConfig,
    TestDuration,
};
use reqwest::Client;
use std::time::{Duration, Instant};

const WARMUP_URL: &str = "https://cachefly.cachefly.net/1mb.test";
const MID_URL: &str = "https://cachefly.cachefly.net/10mb.test";
const LARGE_URL: &str = "https://cachefly.cachefly.net/100mb.test";
/// Same file as [`WARMUP_URL`] — kept as its own named constant to track the
/// TS reference's distinct semantic role (dedicated latency ladder target).
const LATENCY_URL: &str = "https://cachefly.cachefly.net/1mb.test";

// ── Download tuning (mirrors cachefly-provider.ts) ─────────────────────────
const WARMUP_REQUESTS: u32 = 2;
const MIN_MID_TIER_REQUESTS: u32 = 2;
const MAX_MID_TIER_REQUESTS: u32 = 6;
/// Safety valve; never expected to bind.
const MAX_LARGE_TIER_REQUESTS: u32 = 500;
/// "~2s of transfer at the last measured rate" per METHODOLOGY.md §5 step 1.
const TARGET_REQUEST_SECS: f64 = 2.0;
/// Floor for the adaptively-ranged 100mb.test requests.
const RANGE_MIN_BYTES: u64 = 2_000_000;
/// Ceiling — safely under the nominal 100MB (decimal) / 100MiB (binary) file either way.
const RANGE_MAX_BYTES: u64 = 100_000_000;
/// Throughput tick cadence — independent of request/tier boundaries.
const SAMPLE_TICK_MS: f64 = 250.0;
const MAX_CONSECUTIVE_FAILURES: u32 = 3;
/// Per-request safety timeout (matches the TS reference's identical constant,
/// shared with `vultr.rs`'s per-chunk timeout).
const DOWNLOAD_REQUEST_TIMEOUT: Duration = Duration::from_secs(15);
const RETRY_BACKOFF: Duration = Duration::from_millis(200);
/// No pinned value in METHODOLOGY.md for 'auto'; matches the TS reference's
/// identical choice for the same download-only shape.
const DEFAULT_DOWNLOAD_SECS: u64 = 15;

// ── Latency tuning ──────────────────────────────────────────────────────────
const LATENCY_PROBES: u32 = 10;
const LATENCY_WARMUP_DISCARD: usize = 2;
const LATENCY_PROBE_TIMEOUT: Duration = Duration::from_secs(5);

/// Run the CacheFly speed test: latency, then the warm-up/mid/large download ladder.
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

    // ── Latency: 10 HEAD probes (Range: bytes=0-0), discard first 2, min RTT ──
    progress(Phase::CfyLatency, 0.0);

    let mut rtt_samples: Vec<f64> = Vec::with_capacity(LATENCY_PROBES as usize);
    for i in 0..LATENCY_PROBES {
        let frac = i as f64 / LATENCY_PROBES as f64;
        progress(Phase::CfyLatency, frac);
        if let Some(rtt) = probe_latency_once(&client, LATENCY_URL).await {
            rtt_samples.push(rtt);
        }
    }
    progress(Phase::CfyLatency, 1.0);

    // Only discard the warm-up probes when there's enough to discard AND still
    // have something left; otherwise keep whatever came back rather than
    // collapsing to zero samples (mirrors `cachefly-provider.ts` exactly —
    // note this differs from the plain `2.min(len)` clamp-slice used by the
    // sibling v3 providers, which would zero out the sample set here when
    // `len <= LATENCY_WARMUP_DISCARD`).
    let measured_rtts: &[f64] = if rtt_samples.len() > LATENCY_WARMUP_DISCARD {
        &rtt_samples[LATENCY_WARMUP_DISCARD..]
    } else {
        &rtt_samples
    };
    let has_latency = !measured_rtts.is_empty();
    let ping_ms = measured_rtts
        .iter()
        .copied()
        .min_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let jitter_ms = (measured_rtts.len() >= 2).then(|| statistics::jitter_rfc3550(measured_rtts));

    // ── Download: 1mb×2 warm-up, 10mb×N, 100mb×N adaptive Range ─────────────
    progress(Phase::CfyDownload, 0.0);

    let total_secs = match &config.duration {
        TestDuration::Seconds(s) => *s,
        TestDuration::Auto => DEFAULT_DOWNLOAD_SECS,
    };
    let total_secs_f = total_secs as f64;
    let phase_start = Instant::now();
    let deadline = phase_start + Duration::from_secs(total_secs);

    let mut last_request_mbps: f64 = 0.0;
    let mut total_download_bytes: u64 = 0;

    // Warm-up — 2× 1MB requests, whole-request timing only. Discarded from
    // `raw_samples`; used solely to seed the first adaptive rate estimate.
    for _ in 0..WARMUP_REQUESTS {
        if Instant::now() >= deadline {
            break;
        }
        if let Some(resp) = raw_get(&client, WARMUP_URL, None, DOWNLOAD_REQUEST_TIMEOUT).await {
            if let Some((bytes, elapsed_ms)) = time_whole_response(resp).await {
                last_request_mbps = bytes_to_mbps(bytes as f64, elapsed_ms);
                total_download_bytes += bytes;
                let elapsed = phase_start.elapsed().as_secs_f64();
                progress(Phase::CfyDownload, (elapsed / total_secs_f).min(1.0));
            }
        }
    }

    // Mid + large tiers share one continuous 250ms ticker — see `DownloadTicker`.
    let mut raw_samples: Vec<f64> = Vec::new();
    let mut ticker = DownloadTicker::new(Instant::now());
    let mut report_tick = |_mbps: f64| {
        let elapsed = phase_start.elapsed().as_secs_f64();
        progress(Phase::CfyDownload, (elapsed / total_secs_f).min(1.0));
    };

    // Mid tier — 10MB requests. Always at least MIN_MID_TIER_REQUESTS for a
    // stable reading, then escalate to the adaptively-ranged 100MB tier once a
    // full 10MB transfer undershoots the ~2s target at the last measured rate.
    let mut mid_requests: u32 = 0;
    let mut mid_failures: u32 = 0;
    while Instant::now() < deadline && mid_requests < MAX_MID_TIER_REQUESTS {
        let Some(resp) = raw_get(&client, MID_URL, None, DOWNLOAD_REQUEST_TIMEOUT).await else {
            mid_failures += 1;
            if mid_failures >= MAX_CONSECUTIVE_FAILURES {
                break;
            }
            tokio::time::sleep(RETRY_BACKOFF).await;
            continue;
        };
        mid_requests += 1;
        let Some((bytes, elapsed_ms)) =
            drain_into_ticker(resp, &mut ticker, &mut raw_samples, &mut report_tick).await
        else {
            mid_failures += 1;
            if mid_failures >= MAX_CONSECUTIVE_FAILURES {
                break;
            }
            continue;
        };
        mid_failures = 0;
        last_request_mbps = bytes_to_mbps(bytes as f64, elapsed_ms);
        total_download_bytes += bytes;

        if mid_requests >= MIN_MID_TIER_REQUESTS && elapsed_ms < TARGET_REQUEST_SECS * 1000.0 {
            break;
        }
    }

    // Large tier — 100mb.test, `Range: bytes=0-{N-1}` sized to ~2s at the last
    // measured rate (always from byte 0). Fills the remainder of the budget.
    let mut large_requests: u32 = 0;
    let mut large_failures: u32 = 0;
    while Instant::now() < deadline && large_requests < MAX_LARGE_TIER_REQUESTS {
        let range_bytes = adaptive_chunk_bytes(
            last_request_mbps,
            TARGET_REQUEST_SECS,
            RANGE_MIN_BYTES,
            RANGE_MAX_BYTES,
        );
        let Some(resp) = raw_get(
            &client,
            LARGE_URL,
            Some(range_bytes),
            DOWNLOAD_REQUEST_TIMEOUT,
        )
        .await
        else {
            large_failures += 1;
            if large_failures >= MAX_CONSECUTIVE_FAILURES {
                break;
            }
            tokio::time::sleep(RETRY_BACKOFF).await;
            continue;
        };
        large_requests += 1;
        let Some((bytes, elapsed_ms)) =
            drain_into_ticker(resp, &mut ticker, &mut raw_samples, &mut report_tick).await
        else {
            large_failures += 1;
            if large_failures >= MAX_CONSECUTIVE_FAILURES {
                break;
            }
            continue;
        };
        large_failures = 0;
        last_request_mbps = bytes_to_mbps(bytes as f64, elapsed_ms);
        total_download_bytes += bytes;
    }

    ticker.flush(Instant::now(), true, &mut raw_samples, &mut report_tick); // trailing partial bucket
    let download_duration_s = phase_start.elapsed().as_secs_f64();

    // Honest failure: reject rather than resolve with fabricated zeros
    // (METHODOLOGY.md §7's "never fabricated" ethos, applied here to
    // throughput as well as packet loss) — matches the TS reference and every
    // sibling Rust provider's `error_result` convention.
    if !has_latency && raw_samples.is_empty() && total_download_bytes == 0 {
        return Err("CacheFly: no successful latency or download transfers".to_string());
    }

    // ── Point estimate (METHODOLOGY.md §5 steps 2/3/5/6, minus bootstrap — the
    // circular block bootstrap variance is the cross-provider orchestrator's
    // job; see the module doc comment) ──────────────────────────────────────
    let after_plateau: Vec<f64> = if raw_samples.is_empty() {
        Vec::new()
    } else {
        raw_samples[statistics::plateau_start(&raw_samples).min(raw_samples.len())..].to_vec()
    };
    let iqr_basis = if after_plateau.is_empty() {
        raw_samples.clone()
    } else {
        after_plateau
    };
    let cleaned = statistics::filter_outliers_iqr(&iqr_basis, 1.5);
    let basis = if cleaned.is_empty() {
        iqr_basis
    } else {
        cleaned
    };
    let download_speed = if basis.is_empty() {
        0.0
    } else {
        statistics::modified_trimean(&basis)
    };

    // Hodges-Lehmann cross-check (METHODOLOGY.md §5 step 6): a diagnostic-only
    // instability signal, never used to adjust `download_speed` itself
    // (matching the pinned v4 methodology exactly). `ProviderResult` has no
    // field yet to surface this — see the final report for the suggested
    // addition (the same kind of schema gap the TS reference itself flags for
    // its `uploadSupported` field, which also isn't on its provider interface).
    let hl = if basis.is_empty() {
        0.0
    } else {
        statistics::hodges_lehmann(&basis)
    };
    let _unstable = download_speed > 0.0 && (hl - download_speed).abs() / download_speed > 0.15;

    progress(Phase::CfyDownload, 1.0);

    Ok(ProviderResult {
        provider: "CacheFly".to_string(),
        server: "CacheFly CDN".to_string(),
        location: None,
        ping_ms,
        jitter_ms,
        download_mbps: if raw_samples.is_empty() {
            None
        } else {
            Some(download_speed)
        },
        // CacheFly never runs an upload phase (no upload endpoint exists at
        // all). `None` here is deliberate — an ABSENT measurement, not a
        // measured zero. `upload_bytes`/`upload_duration_s` stay 0/0.0 (those
        // are literal counts, correctly zero) and `bandwidth_samples.upload`
        // stays empty, which already makes `aggregate`'s MIN_MERGE_SAMPLES
        // gate correctly exclude CacheFly from the upload merge with zero
        // orchestrator changes.
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

// ── HTTP helpers ────────────────────────────────────────────────────────────

/// Cache-busting query string, mirroring the TS providers' `?_t=...&_r=...`
/// convention (no `rand` crate dependency — wall-clock nanoseconds give ample
/// uniqueness for this purpose across successive calls).
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

/// GET with an optional `Range: bytes=0-{range_bytes-1}` header and a
/// cache-busting query string. Returns `None` on any transport error or a
/// non-success status — never treated as fatal by callers (mirrors
/// `cachefly-provider.ts`'s `rawGet`, which never throws).
async fn raw_get(
    client: &Client,
    url: &str,
    range_bytes: Option<u64>,
    timeout: Duration,
) -> Option<reqwest::Response> {
    let mut req = client.get(cache_bust_url(url)).timeout(timeout);
    if let Some(n) = range_bytes {
        if n > 0 {
            req = req.header("Range", format!("bytes=0-{}", n.saturating_sub(1)));
        }
    }
    match req.send().await {
        Ok(resp) if resp.status().is_success() => Some(resp),
        _ => None,
    }
}

/// Whole-response timing with no interval bucketing — used only for the fixed
/// 1MB warm-up requests, which exist solely to seed the first adaptive rate
/// estimate and never contribute to `raw_samples`.
async fn time_whole_response(resp: reqwest::Response) -> Option<(u64, f64)> {
    let start = Instant::now();
    match resp.bytes().await {
        Ok(buf) if !buf.is_empty() => {
            Some((buf.len() as u64, start.elapsed().as_secs_f64() * 1000.0))
        }
        _ => None,
    }
}

/// Single `HEAD` probe with `Range: bytes=0-0` — measures RTT without a body.
/// Wall-clock only: unlike a browser, reqwest exposes no `PerformanceResourceTiming`-
/// style TTFB breakdown, and (per the TS reference's own note) that finer
/// timing has no practical effect for these edge hosts anyway.
async fn probe_latency_once(client: &Client, url: &str) -> Option<f64> {
    let start = Instant::now();
    client
        .head(cache_bust_url(url))
        .header("Range", "bytes=0-0")
        .timeout(LATENCY_PROBE_TIMEOUT)
        .send()
        .await
        .ok()?;
    Some(start.elapsed().as_secs_f64() * 1000.0)
}

/// A 250ms throughput ticker whose clock runs continuously across request AND
/// tier boundaries (mirrors `cachefly-provider.ts`'s `makeDownloadTicker`) —
/// pushes a sample into `raw_samples` (and calls `on_tick`) only when a real
/// ~250ms interval has elapsed, so a request boundary never manufactures a
/// spurious tiny tail sample partway through a bucket.
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

    /// Emit a sample once a real ~250ms interval has elapsed since the last
    /// flush, or unconditionally when `force` is true (the trailing partial
    /// bucket at the very end of the download phase).
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

/// Streams `resp`'s body into `ticker` (adding bytes + periodic 250ms
/// flushes) via [`reqwest::Response::chunk`] (no `stream` Cargo feature
/// needed). Returns the WHOLE request's own `(bytes, elapsed_ms)` — used only
/// to seed the next adaptive request size — or `None` if nothing was read.
async fn drain_into_ticker(
    mut resp: reqwest::Response,
    ticker: &mut DownloadTicker,
    raw_samples: &mut Vec<f64>,
    on_tick: &mut dyn FnMut(f64),
) -> Option<(u64, f64)> {
    let req_start = Instant::now();
    let mut bytes: u64 = 0;
    loop {
        match resp.chunk().await {
            Ok(Some(chunk)) => {
                let n = chunk.len() as u64;
                if n > 0 {
                    bytes += n;
                    ticker.add_bytes(n);
                }
                ticker.flush(Instant::now(), false, raw_samples, on_tick);
            }
            Ok(None) => break,
            Err(_) => break, // connection error mid-flight — report what was transferred
        }
    }
    let elapsed_ms = req_start.elapsed().as_secs_f64() * 1000.0;
    (bytes > 0).then_some((bytes, elapsed_ms))
}

/// Build an error `ProviderResult` with zeroed metrics.
fn error_result(msg: String) -> ProviderResult {
    ProviderResult {
        provider: "CacheFly".to_string(),
        server: "CacheFly CDN".to_string(),
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
    fn bytes_to_mbps_matches_hand_computed() {
        // 10,000,000 bytes in 1000ms = 80 Mbps.
        assert!((bytes_to_mbps(10_000_000.0, 1000.0) - 80.0).abs() < 1e-9);
    }

    #[test]
    fn bytes_to_mbps_guards_non_positive_inputs() {
        assert_eq!(bytes_to_mbps(0.0, 1000.0), 0.0);
        assert_eq!(bytes_to_mbps(100.0, 0.0), 0.0);
        assert_eq!(bytes_to_mbps(-5.0, 1000.0), 0.0);
    }

    #[test]
    fn ticker_flushes_only_after_real_interval() {
        let start = Instant::now();
        let mut ticker = DownloadTicker::new(start);
        let mut samples = Vec::new();
        ticker.add_bytes(1_000_000);
        // Not enough wall-clock time has passed yet (force=false) — no sample.
        ticker.flush(start, false, &mut samples, &mut |_| {});
        assert!(samples.is_empty());

        // A forced flush emits the trailing partial bucket regardless.
        let later = start + Duration::from_millis(10);
        ticker.flush(later, true, &mut samples, &mut |_| {});
        assert_eq!(samples.len(), 1);
    }

    #[test]
    fn error_result_has_no_measurements() {
        let r = error_result("boom".to_string());
        assert_eq!(r.provider, "CacheFly");
        assert!(r.error.is_some());
        assert!(r.download_mbps.is_none());
        assert!(r.upload_mbps.is_none());
        assert_eq!(r.download_bytes, 0);
    }
}
