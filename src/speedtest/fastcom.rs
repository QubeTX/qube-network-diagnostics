use super::adaptive::adaptive_chunk_bytes;
use super::{statistics, BandwidthSamples, Phase, ProviderResult, SpeedTestConfig, TestDuration};
use reqwest::Client;
use std::time::{Duration, Instant};

/// Netflix fast.com API endpoint.
const FAST_API_URL: &str = "https://api.fast.com/netflix/speedtest/v2";

/// Fallback token (may expire if Netflix rotates it).
const FALLBACK_TOKEN: &str = "YXNkZmFzZGxmbnNkYWZoYXNkZmhrYWxm";

/// Number of OCA server URLs to request.
const URL_COUNT: u32 = 5;

/// Default auto-mode download duration (seconds) — fast.com's natural cycle.
const AUTO_DOWNLOAD_SECS: u64 = 15;

/// Default auto-mode upload duration (seconds).
const AUTO_UPLOAD_SECS: u64 = 10;

/// Latency probes against the nearest OCA (first two discarded as TCP/TLS
/// warmup — the same shape as the Cloudflare/LibreSpeed probes).
const LATENCY_PROBES: usize = 10;
const LATENCY_WARMUP_DISCARD: usize = 2;

/// Adaptive upload sizing (see `adaptive::adaptive_chunk_bytes`).
const TARGET_REQUEST_SECS: f64 = 2.0;
const UPLOAD_MIN_BYTES: u64 = 256 * 1024;
const UPLOAD_MAX_BYTES: u64 = 16_000_000;

/// Consecutive POST failures before concluding the OCA doesn't accept
/// uploads. After the first two failures one small-payload retry is attempted
/// (some OCAs reject large POSTs but accept small ones).
const UPLOAD_GIVE_UP_AFTER: u32 = 5;

/// Minimum per-request timeout floor (see cloudflare.rs::remaining_budget).
const MIN_REQUEST_TIMEOUT: Duration = Duration::from_secs(1);

/// Time left until `deadline`, floored at `MIN_REQUEST_TIMEOUT`. Used as the
/// per-request `.timeout()` so a stalled transfer can't outlive the phase
/// (the 120s client timeout would otherwise dominate the deadline loop).
fn remaining_budget(deadline: Instant) -> Duration {
    deadline
        .saturating_duration_since(Instant::now())
        .max(MIN_REQUEST_TIMEOUT)
}

/// Run the fast.com (Netflix) speed test: server discovery, download, optional upload.
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

    // ── Server discovery ─────────────────────────────────────────────
    progress(Phase::FcDiscovery, 0.0);

    // Dynamically extract the API token from fast.com's JS bundle
    let token = extract_token(&client)
        .await
        .unwrap_or_else(|_| FALLBACK_TOKEN.to_string());

    progress(Phase::FcDiscovery, 0.3);

    let api_url = format!(
        "{}?https=true&token={}&urlCount={}",
        FAST_API_URL, token, URL_COUNT
    );

    let resp = client
        .get(&api_url)
        .timeout(Duration::from_secs(15))
        .send()
        .await
        .map_err(|e| format!("fast.com discovery failed: {e}"))?;

    if !resp.status().is_success() {
        return Err(format!("fast.com API returned status {}", resp.status()));
    }

    let body: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| format!("fast.com discovery parse error: {e}"))?;

    // Extract OCA URLs from the response
    let targets = body["targets"]
        .as_array()
        .or_else(|| body.as_array())
        .ok_or("fast.com: no targets in response")?;

    let urls: Vec<String> = targets
        .iter()
        .filter_map(|t| t["url"].as_str().map(|s| s.to_string()))
        .collect();

    if urls.is_empty() {
        return Err("fast.com: no download URLs found".to_string());
    }

    // Extract server location from first target
    let location = targets
        .first()
        .and_then(|t| t["location"]["city"].as_str())
        .map(|s| s.to_string());

    progress(Phase::FcDiscovery, 1.0);

    // ── Duration ─────────────────────────────────────────────────────
    let (dl_secs, ul_secs) = match &config.fastcom_duration {
        TestDuration::Seconds(s) => (*s, *s),
        TestDuration::Auto => (AUTO_DOWNLOAD_SECS, AUTO_UPLOAD_SECS),
    };

    // ── Download phase ───────────────────────────────────────────────
    progress(Phase::FcDownload, 0.0);

    let dl_deadline = Instant::now() + Duration::from_secs(dl_secs);
    let mut dl_bytes: u64 = 0;
    let dl_start = Instant::now();
    let mut dl_mbps_samples: Vec<f64> = Vec::new();
    let mut url_idx = 0;

    // Measure latency with a burst of tiny range requests (warmup discarded,
    // min of the rest), matching the Cloudflare/LibreSpeed probe shape — the
    // old single probe was so noisy it could distort the aggregate latency
    // fallback, and jitter was never measured at all.
    let mut ping_ms: Option<f64> = None;
    let mut jitter_ms: Option<f64> = None;
    if let Some(first_url) = urls.first() {
        let ping_url = format!("{}&bytes=1", first_url);
        let mut rtts: Vec<f64> = Vec::with_capacity(LATENCY_PROBES);
        for _ in 0..LATENCY_PROBES {
            let ping_start = Instant::now();
            if client
                .head(&ping_url)
                .timeout(Duration::from_secs(5))
                .send()
                .await
                .is_ok()
            {
                rtts.push(ping_start.elapsed().as_secs_f64() * 1000.0);
            }
        }
        let warmup_skip = LATENCY_WARMUP_DISCARD.min(rtts.len());
        let trimmed = &rtts[warmup_skip..];
        ping_ms = trimmed
            .iter()
            .copied()
            .min_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        if trimmed.len() >= 2 {
            jitter_ms = Some(statistics::jitter_rfc3550(trimmed));
        }
    }

    while Instant::now() < dl_deadline {
        let url = &urls[url_idx % urls.len()];
        url_idx += 1;

        let req_start = Instant::now();
        match client
            .get(url)
            .timeout(remaining_budget(dl_deadline))
            .send()
            .await
        {
            Ok(resp) if resp.status().is_success() => {
                if let Ok(body) = resp.bytes().await {
                    let req_bytes = body.len() as u64;
                    let req_duration = req_start.elapsed().as_secs_f64();
                    dl_bytes += req_bytes;
                    if req_duration > 0.0 {
                        dl_mbps_samples
                            .push((req_bytes as f64 * 8.0) / (req_duration * 1_000_000.0));
                    }

                    let elapsed = dl_start.elapsed().as_secs_f64();
                    let frac = (elapsed / dl_secs as f64).min(1.0);
                    progress(Phase::FcDownload, frac);
                }
            }
            Err(_) => {}
            _ => {}
        }
    }

    let dl_elapsed = dl_start.elapsed().as_secs_f64();
    progress(Phase::FcDownload, 1.0);

    let download_mbps = if dl_mbps_samples.is_empty() {
        None
    } else {
        Some(statistics::accurate_bandwidth(&dl_mbps_samples))
    };

    // ── Upload phase ─────────────────────────────────────────────────
    // fast.com upload is less documented; attempt POST to the same URLs
    progress(Phase::FcUpload, 0.0);

    let ul_deadline = Instant::now() + Duration::from_secs(ul_secs);
    let mut ul_bytes: u64 = 0;
    let ul_start = Instant::now();
    let mut ul_mbps_samples: Vec<f64> = Vec::new();
    url_idx = 0;

    // Adaptive payload sizing; on early consecutive failures, retry once with
    // the minimum payload before counting further failures — some OCAs reject
    // large POSTs but accept small ones.
    let mut ul_chunk_bytes: u64 = UPLOAD_MIN_BYTES.max(1_000_000);
    let mut upload_payload = vec![0u8; ul_chunk_bytes as usize];
    let mut upload_failures = 0u32;
    let mut tried_small_payload = false;
    while Instant::now() < ul_deadline {
        let url = &urls[url_idx % urls.len()];
        url_idx += 1;

        if upload_payload.len() as u64 != ul_chunk_bytes {
            upload_payload = vec![0u8; ul_chunk_bytes as usize];
        }
        let req_bytes = upload_payload.len() as u64;
        let req_start = Instant::now();
        match client
            .post(url)
            .body(upload_payload.clone())
            // Bound by the remaining phase budget (capped at the original 30s)
            // so a stalled upload near the deadline can't run far past it.
            .timeout(remaining_budget(ul_deadline).min(Duration::from_secs(30)))
            .send()
            .await
        {
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
                progress(Phase::FcUpload, frac);
                upload_failures = 0; // reset on success
            }
            _ => {
                upload_failures += 1;
                if upload_failures >= 2 && !tried_small_payload {
                    tried_small_payload = true;
                    ul_chunk_bytes = UPLOAD_MIN_BYTES;
                    continue;
                }
                if upload_failures >= UPLOAD_GIVE_UP_AFTER {
                    break;
                }
            }
        }
    }

    let ul_elapsed = ul_start.elapsed().as_secs_f64();
    progress(Phase::FcUpload, 1.0);

    let upload_mbps = if ul_mbps_samples.is_empty() {
        None
    } else {
        Some(statistics::accurate_upload_bandwidth(&ul_mbps_samples))
    };

    // Honest failure: discovery succeeded but neither direction produced a
    // usable transfer sample. fast.com upload is best-effort (often unsupported),
    // so this fires when the download also collected nothing. Report an explicit
    // error so the speedqx table shows an error row (and aggregation excludes it).
    let error = if dl_mbps_samples.is_empty() && ul_mbps_samples.is_empty() {
        Some("no successful transfers".to_string())
    } else {
        None
    };

    Ok(ProviderResult {
        provider: "fast.com".to_string(),
        server: urls
            .first()
            .cloned()
            .unwrap_or_else(|| "unknown".to_string()),
        location,
        ping_ms,
        jitter_ms,
        download_mbps,
        upload_mbps,
        download_bytes: dl_bytes,
        upload_bytes: ul_bytes,
        download_duration_s: dl_elapsed,
        upload_duration_s: ul_elapsed,
        packet_loss_pct: None,
        error,
        bandwidth_samples: Some(BandwidthSamples {
            download: dl_mbps_samples,
            upload: ul_mbps_samples,
        }),
    })
}

/// Dynamically extract the API token from fast.com's JS bundle.
/// 1. Fetch https://fast.com/
/// 2. Find the JS filename (app-XXXX.js)
/// 3. Fetch the JS and extract token:"..." value
async fn extract_token(client: &Client) -> Result<String, String> {
    // Step 1: Fetch fast.com HTML
    let html = client
        .get("https://fast.com/")
        .timeout(Duration::from_secs(10))
        .send()
        .await
        .map_err(|e| format!("fast.com page fetch failed: {e}"))?
        .text()
        .await
        .map_err(|e| format!("fast.com page read failed: {e}"))?;

    // Step 2: Find JS bundle filename (e.g., "app-0bffe1.js")
    let js_filename = html
        .split("app-")
        .nth(1)
        .and_then(|s| s.split('"').next())
        .and_then(|s| s.split('\'').next())
        .map(|s| format!("app-{}", s))
        .ok_or("fast.com: could not find JS bundle filename")?;

    // Ensure filename ends with .js
    let js_filename = if js_filename.contains(".js") {
        js_filename
            .split(".js")
            .next()
            .unwrap_or(&js_filename)
            .to_string()
            + ".js"
    } else {
        return Err("fast.com: invalid JS filename".to_string());
    };

    // Step 3: Fetch the JS bundle
    let js_url = format!("https://fast.com/{}", js_filename);
    let js = client
        .get(&js_url)
        .timeout(Duration::from_secs(10))
        .send()
        .await
        .map_err(|e| format!("fast.com JS fetch failed: {e}"))?
        .text()
        .await
        .map_err(|e| format!("fast.com JS read failed: {e}"))?;

    // Step 4: Extract token value from `token:"VALUE"` pattern
    let token = js
        .split("token:")
        .nth(1)
        .and_then(|s| {
            let s = s.trim_start();
            if let Some(stripped) = s.strip_prefix('"') {
                stripped.split('"').next().map(|t| t.to_string())
            } else if let Some(stripped) = s.strip_prefix('\'') {
                stripped.split('\'').next().map(|t| t.to_string())
            } else {
                None
            }
        })
        .ok_or("fast.com: could not extract token from JS bundle")?;

    if token.is_empty() {
        return Err("fast.com: extracted empty token".to_string());
    }

    Ok(token)
}

fn error_result(msg: String) -> ProviderResult {
    ProviderResult {
        provider: "fast.com".to_string(),
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
