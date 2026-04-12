use super::{BandwidthSamples, Phase, ProviderResult, SpeedTestConfig, TestDuration, statistics};
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

/// Upload chunk size (4 MB).
const UPLOAD_CHUNK_SIZE: usize = 4_000_000;

/// Run the fast.com (Netflix) speed test: server discovery, download, optional upload.
pub async fn run<F>(config: &SpeedTestConfig, progress: F) -> ProviderResult
where
    F: Fn(Phase, f64) + Send + Sync,
{
    match run_inner(config, &progress).await {
        Ok(result) => result,
        Err(e) => error_result(format!("{e}")),
    }
}

async fn run_inner<F>(
    config: &SpeedTestConfig,
    progress: &F,
) -> Result<ProviderResult, String>
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
    let token = extract_token(&client).await.unwrap_or_else(|_| FALLBACK_TOKEN.to_string());

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
        return Err(format!(
            "fast.com API returned status {}",
            resp.status()
        ));
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

    // Measure latency with a small initial request (1 byte range)
    let mut ping_ms: Option<f64> = None;
    if let Some(first_url) = urls.first() {
        // Use a tiny range request to measure connection RTT, not download time
        let ping_url = format!("{}&bytes=1", first_url);
        let ping_start = Instant::now();
        if client.head(&ping_url).timeout(Duration::from_secs(5)).send().await.is_ok() {
            ping_ms = Some(ping_start.elapsed().as_secs_f64() * 1000.0);
        }
    }

    while Instant::now() < dl_deadline {
        let url = &urls[url_idx % urls.len()];
        url_idx += 1;

        let req_start = Instant::now();
        match client.get(url).send().await {
            Ok(resp) => match resp.bytes().await {
                Ok(body) => {
                    let req_bytes = body.len() as u64;
                    let req_duration = req_start.elapsed().as_secs_f64();
                    dl_bytes += req_bytes;
                    if req_duration > 0.0 {
                        dl_mbps_samples.push((req_bytes as f64 * 8.0) / (req_duration * 1_000_000.0));
                    }

                    let elapsed = dl_start.elapsed().as_secs_f64();
                    let frac = (elapsed / dl_secs as f64).min(1.0);
                    progress(Phase::FcDownload, frac);
                }
                Err(_) => {}
            },
            Err(_) => {}
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

    let upload_payload = vec![0u8; UPLOAD_CHUNK_SIZE];
    let ul_deadline = Instant::now() + Duration::from_secs(ul_secs);
    let mut ul_bytes: u64 = 0;
    let ul_start = Instant::now();
    let mut ul_mbps_samples: Vec<f64> = Vec::new();
    url_idx = 0;

    let mut upload_failures = 0u32;
    while Instant::now() < ul_deadline {
        let url = &urls[url_idx % urls.len()];
        url_idx += 1;

        let req_start = Instant::now();
        match client
            .post(url)
            .body(upload_payload.clone())
            .timeout(Duration::from_secs(30))
            .send()
            .await
        {
            Ok(resp) if resp.status().is_success() || resp.status().is_redirection() => {
                let req_duration = req_start.elapsed().as_secs_f64();
                ul_bytes += UPLOAD_CHUNK_SIZE as u64;
                if req_duration > 0.0 {
                    ul_mbps_samples.push((UPLOAD_CHUNK_SIZE as f64 * 8.0) / (req_duration * 1_000_000.0));
                }
                let elapsed = ul_start.elapsed().as_secs_f64();
                let frac = (elapsed / ul_secs as f64).min(1.0);
                progress(Phase::FcUpload, frac);
                upload_failures = 0; // reset on success
            }
            _ => {
                upload_failures += 1;
                // If 3+ consecutive failures, upload not supported — stop trying
                if upload_failures >= 3 {
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

    Ok(ProviderResult {
        provider: "fast.com".to_string(),
        server: urls.first().cloned().unwrap_or_else(|| "unknown".to_string()),
        location,
        ping_ms,
        jitter_ms: None, // fast.com doesn't provide jitter data
        download_mbps,
        upload_mbps,
        download_bytes: dl_bytes,
        upload_bytes: ul_bytes,
        download_duration_s: dl_elapsed,
        upload_duration_s: ul_elapsed,
        packet_loss_pct: None,
        error: None,
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
        js_filename.split(".js").next().unwrap_or(&js_filename).to_string() + ".js"
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
            if s.starts_with('"') {
                s[1..].split('"').next().map(|t| t.to_string())
            } else if s.starts_with('\'') {
                s[1..].split('\'').next().map(|t| t.to_string())
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
