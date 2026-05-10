use super::{statistics, BandwidthSamples, Phase, ProviderResult, SpeedTestConfig, TestDuration};
use reqwest::Client;
use std::time::{Duration, Instant};

const LATENCY_URL: &str = "https://speed.cloudflare.com/__down?bytes=0";
const DOWNLOAD_URL: &str = "https://speed.cloudflare.com/__down?bytes=10000000";
const UPLOAD_URL: &str = "https://speed.cloudflare.com/__up";

/// Chunk size for upload payloads (2 MB).
const UPLOAD_CHUNK_SIZE: usize = 2_000_000;

/// Run the Cloudflare speed test: latency, download, upload.
pub async fn run<F>(config: &SpeedTestConfig, progress: F) -> ProviderResult
where
    F: Fn(Phase, f64) + Send + Sync,
{
    let client = match Client::builder().timeout(Duration::from_secs(120)).build() {
        Ok(c) => c,
        Err(e) => return error_result(format!("Failed to build HTTP client: {e}")),
    };

    // ── Latency phase ────────────────────────────────────────────────
    progress(Phase::CfLatency, 0.0);

    let probes = config.latency_probes.max(4); // need at least 4 to discard 2
    let mut rtts: Vec<f64> = Vec::with_capacity(probes as usize);
    let mut failures: u32 = 0;

    for i in 0..probes {
        let frac = i as f64 / probes as f64;
        progress(Phase::CfLatency, frac);

        let start = Instant::now();
        match client.head(LATENCY_URL).send().await {
            Ok(_resp) => {
                let rtt_ms = start.elapsed().as_secs_f64() * 1000.0;
                rtts.push(rtt_ms);
            }
            Err(_) => {
                failures += 1;
            }
        }
    }

    progress(Phase::CfLatency, 1.0);

    // Discard first 2 probes (TCP/TLS warmup)
    let warmup_skip = 2.min(rtts.len());
    let trimmed: Vec<f64> = rtts[warmup_skip..].to_vec();

    let (ping_ms, jitter_ms) = if trimmed.is_empty() {
        (None, None)
    } else {
        // Minimum RTT: best represents physical link latency (consistent with NDT7)
        let ping = trimmed
            .iter()
            .copied()
            .min_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal))
            .unwrap_or(0.0);
        let jitter = statistics::jitter_rfc3550(&trimmed);
        (Some(ping), Some(jitter))
    };

    let packet_loss_pct = if probes > 0 {
        Some(failures as f64 / probes as f64 * 100.0)
    } else {
        None
    };

    // ── Duration per direction ──────────────────────────────────────
    // Duration is per direction: 30s means 30s download + 30s upload
    let (dl_secs, ul_secs) = match &config.duration {
        TestDuration::Seconds(s) => (*s, *s),
        TestDuration::Auto => (15, 15),
    };

    // ── Download phase ───────────────────────────────────────────────
    progress(Phase::CfDownload, 0.0);

    let dl_deadline = Instant::now() + Duration::from_secs(dl_secs);
    let mut dl_bytes: u64 = 0;
    let dl_start = Instant::now();
    let mut dl_mbps_samples: Vec<f64> = Vec::new();

    while Instant::now() < dl_deadline {
        let req_start = Instant::now();
        match client.get(DOWNLOAD_URL).send().await {
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
                    progress(Phase::CfDownload, frac);
                }
            }
            Err(_) => {}
            _ => {}
        }
    }

    let dl_elapsed = dl_start.elapsed().as_secs_f64();
    progress(Phase::CfDownload, 1.0);

    let download_mbps = if dl_mbps_samples.is_empty() {
        None
    } else {
        Some(statistics::accurate_bandwidth(&dl_mbps_samples))
    };

    // ── Upload phase ─────────────────────────────────────────────────
    progress(Phase::CfUpload, 0.0);

    let upload_payload = vec![0u8; UPLOAD_CHUNK_SIZE];
    let ul_deadline = Instant::now() + Duration::from_secs(ul_secs);
    let mut ul_bytes: u64 = 0;
    let ul_start = Instant::now();
    let mut ul_mbps_samples: Vec<f64> = Vec::new();

    while Instant::now() < ul_deadline {
        let req_start = Instant::now();
        match client
            .post(UPLOAD_URL)
            .body(upload_payload.clone())
            .send()
            .await
        {
            Ok(resp) if resp.status().is_success() => {
                let req_duration = req_start.elapsed().as_secs_f64();
                ul_bytes += UPLOAD_CHUNK_SIZE as u64;
                if req_duration > 0.0 {
                    ul_mbps_samples
                        .push((UPLOAD_CHUNK_SIZE as f64 * 8.0) / (req_duration * 1_000_000.0));
                }
                let elapsed = ul_start.elapsed().as_secs_f64();
                let frac = (elapsed / ul_secs as f64).min(1.0);
                progress(Phase::CfUpload, frac);
            }
            Err(_) => {}
            _ => {}
        }
    }

    let ul_elapsed = ul_start.elapsed().as_secs_f64();
    progress(Phase::CfUpload, 1.0);

    let upload_mbps = if ul_mbps_samples.is_empty() {
        None
    } else {
        Some(statistics::accurate_upload_bandwidth(&ul_mbps_samples))
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
        error: None,
        bandwidth_samples: Some(BandwidthSamples {
            download: dl_mbps_samples,
            upload: ul_mbps_samples,
        }),
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
    }
}
