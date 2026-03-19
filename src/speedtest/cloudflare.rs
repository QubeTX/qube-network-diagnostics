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
        // Minimum RTT: best represents physical link latency (consistent with NDT7)
        let ping = trimmed
            .iter()
            .copied()
            .min_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal))
            .unwrap_or(0.0);
        let jitter = avg_consecutive_diff(&trimmed);
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
    let mut dl_samples: Vec<(u64, f64)> = Vec::new(); // (bytes, duration) per request

    while Instant::now() < dl_deadline {
        let req_start = Instant::now();
        match client.get(DOWNLOAD_URL).send().await {
            Ok(resp) => match resp.bytes().await {
                Ok(body) => {
                    let req_bytes = body.len() as u64;
                    let req_duration = req_start.elapsed().as_secs_f64();
                    dl_bytes += req_bytes;
                    dl_samples.push((req_bytes, req_duration));
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

    // Slow-start trimming: discard first sample if we have enough
    let download_mbps = compute_trimmed_throughput(&dl_samples, dl_bytes, dl_elapsed);

    // ── Upload phase ─────────────────────────────────────────────────
    progress(Phase::CfUpload, 0.0);

    let upload_payload = vec![0u8; UPLOAD_CHUNK_SIZE];
    let ul_deadline = Instant::now() + Duration::from_secs(ul_secs);
    let mut ul_bytes: u64 = 0;
    let ul_start = Instant::now();
    let mut ul_samples: Vec<(u64, f64)> = Vec::new();

    while Instant::now() < ul_deadline {
        let req_start = Instant::now();
        match client
            .post(UPLOAD_URL)
            .body(upload_payload.clone())
            .send()
            .await
        {
            Ok(_) => {
                let req_duration = req_start.elapsed().as_secs_f64();
                ul_bytes += UPLOAD_CHUNK_SIZE as u64;
                ul_samples.push((UPLOAD_CHUNK_SIZE as u64, req_duration));
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

    let upload_mbps = compute_trimmed_throughput(&ul_samples, ul_bytes, ul_elapsed);

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

/// Compute throughput with slow-start trimming.
/// If there are 4+ samples, discard the first one (TCP slow-start).
fn compute_trimmed_throughput(samples: &[(u64, f64)], total_bytes: u64, total_elapsed: f64) -> Option<f64> {
    if total_elapsed <= 0.0 || total_bytes == 0 {
        return None;
    }

    if samples.len() >= 4 {
        let trimmed_bytes: u64 = samples[1..].iter().map(|(b, _)| *b).sum();
        let trimmed_duration: f64 = samples[1..].iter().map(|(_, d)| *d).sum();
        if trimmed_duration > 0.0 {
            return Some((trimmed_bytes as f64 * 8.0) / (trimmed_duration * 1_000_000.0));
        }
    }

    Some((total_bytes as f64 * 8.0) / (total_elapsed * 1_000_000.0))
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
