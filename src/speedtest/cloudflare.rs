use super::{Phase, ProviderResult, SpeedTestConfig, TestDuration};
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
    let client = match Client::builder()
        .timeout(Duration::from_secs(120))
        .build()
    {
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
        let ping = median(&trimmed);
        let jitter = avg_consecutive_diff(&trimmed);
        (Some(ping), Some(jitter))
    };

    let packet_loss_pct = if probes > 0 {
        Some(failures as f64 / probes as f64 * 100.0)
    } else {
        None
    };

    // ── Duration split ───────────────────────────────────────────────
    let (dl_secs, ul_secs) = match &config.duration {
        TestDuration::Seconds(s) => {
            let dl = s / 2;
            let ul = s - dl;
            (dl, ul)
        }
        TestDuration::Auto => (15, 15),
    };

    // ── Download phase ───────────────────────────────────────────────
    progress(Phase::CfDownload, 0.0);

    let dl_deadline = Instant::now() + Duration::from_secs(dl_secs);
    let mut dl_bytes: u64 = 0;
    let dl_start = Instant::now();

    while Instant::now() < dl_deadline {
        match client.get(DOWNLOAD_URL).send().await {
            Ok(resp) => match resp.bytes().await {
                Ok(body) => {
                    dl_bytes += body.len() as u64;
                    let elapsed = dl_start.elapsed().as_secs_f64();
                    let total_duration = dl_secs as f64;
                    let frac = (elapsed / total_duration).min(1.0);
                    progress(Phase::CfDownload, frac);
                }
                Err(_) => {}
            },
            Err(_) => {}
        }
    }

    let dl_elapsed = dl_start.elapsed().as_secs_f64();
    progress(Phase::CfDownload, 1.0);

    let download_mbps = if dl_elapsed > 0.0 {
        Some((dl_bytes as f64 * 8.0) / (dl_elapsed * 1_000_000.0))
    } else {
        None
    };

    // ── Upload phase ─────────────────────────────────────────────────
    progress(Phase::CfUpload, 0.0);

    let upload_payload = vec![0u8; UPLOAD_CHUNK_SIZE];
    let ul_deadline = Instant::now() + Duration::from_secs(ul_secs);
    let mut ul_bytes: u64 = 0;
    let ul_start = Instant::now();

    while Instant::now() < ul_deadline {
        match client
            .post(UPLOAD_URL)
            .body(upload_payload.clone())
            .send()
            .await
        {
            Ok(_) => {
                ul_bytes += UPLOAD_CHUNK_SIZE as u64;
                let elapsed = ul_start.elapsed().as_secs_f64();
                let total_duration = ul_secs as f64;
                let frac = (elapsed / total_duration).min(1.0);
                progress(Phase::CfUpload, frac);
            }
            Err(_) => {}
        }
    }

    let ul_elapsed = ul_start.elapsed().as_secs_f64();
    progress(Phase::CfUpload, 1.0);

    let upload_mbps = if ul_elapsed > 0.0 {
        Some((ul_bytes as f64 * 8.0) / (ul_elapsed * 1_000_000.0))
    } else {
        None
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
    }
}

/// Compute the median of a slice. The slice is cloned and sorted internally.
fn median(values: &[f64]) -> f64 {
    let mut sorted = values.to_vec();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let n = sorted.len();
    if n == 0 {
        return 0.0;
    }
    if n % 2 == 1 {
        sorted[n / 2]
    } else {
        (sorted[n / 2 - 1] + sorted[n / 2]) / 2.0
    }
}

/// Average absolute difference between consecutive samples.
fn avg_consecutive_diff(values: &[f64]) -> f64 {
    if values.len() < 2 {
        return 0.0;
    }
    let sum: f64 = values
        .windows(2)
        .map(|w| (w[1] - w[0]).abs())
        .sum();
    sum / (values.len() - 1) as f64
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
    }
}
