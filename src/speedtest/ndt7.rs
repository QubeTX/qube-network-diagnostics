use super::{Phase, ProviderResult, SpeedTestConfig, TestDuration};
use futures_util::{SinkExt, StreamExt};
use std::time::{Duration, Instant};
use tokio_tungstenite::tungstenite::Message;

const LOCATE_URL: &str =
    "https://locate.measurementlab.net/v2/nearest/ndt/ndt7";

/// Upload frame size (8 KB).
const UPLOAD_FRAME_SIZE: usize = 8192;

/// Maximum duration to wait for a single download or upload test (safety cap).
const SINGLE_TEST_TIMEOUT: Duration = Duration::from_secs(30);

/// Run the NDT7 (M-Lab) speed test: server discovery, download, upload.
pub async fn run<F>(config: &SpeedTestConfig, progress: F) -> ProviderResult
where
    F: Fn(Phase, f64) + Send + Sync,
{
    match run_inner(config, &progress).await {
        Ok(result) => result,
        Err(e) => error_result(format!("{e}")),
    }
}

/// Internal implementation that propagates errors.
async fn run_inner<F>(
    config: &SpeedTestConfig,
    progress: &F,
) -> Result<ProviderResult, String>
where
    F: Fn(Phase, f64) + Send + Sync,
{
    // ── Server discovery ─────────────────────────────────────────────
    progress(Phase::Ndt7Discovery, 0.0);

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(15))
        .build()
        .map_err(|e| format!("HTTP client error: {e}"))?;

    let resp = client
        .get(LOCATE_URL)
        .send()
        .await
        .map_err(|e| format!("NDT7 discovery failed: {e}"))?;

    let body: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| format!("NDT7 discovery parse error: {e}"))?;

    let results = body["results"]
        .as_array()
        .ok_or("NDT7 discovery: missing results array")?;

    let server_entry = results
        .first()
        .ok_or("NDT7 discovery: no servers found")?;

    let machine = server_entry["machine"]
        .as_str()
        .unwrap_or("unknown")
        .to_string();

    let city = server_entry["location"]["city"]
        .as_str()
        .unwrap_or("");
    let country = server_entry["location"]["country"]
        .as_str()
        .unwrap_or("");
    let location = if !city.is_empty() || !country.is_empty() {
        Some(format!(
            "{}{}{}",
            city,
            if !city.is_empty() && !country.is_empty() { ", " } else { "" },
            country
        ))
    } else {
        None
    };

    let urls = &server_entry["urls"];
    let download_url = urls["wss:///ndt/v7/download"]
        .as_str()
        .ok_or("NDT7 discovery: missing download URL")?
        .to_string();
    let upload_url = urls["wss:///ndt/v7/upload"]
        .as_str()
        .ok_or("NDT7 discovery: missing upload URL")?
        .to_string();

    progress(Phase::Ndt7Discovery, 1.0);

    // ── Determine iteration count ────────────────────────────────────
    let iterations = match &config.duration {
        TestDuration::Seconds(s) if *s > 20 => (*s as f64 / 20.0).ceil() as u32,
        TestDuration::Seconds(_) => 1,
        TestDuration::Auto => 1,
    };

    let mut all_download_mbps: Vec<f64> = Vec::new();
    let mut all_upload_mbps: Vec<f64> = Vec::new();
    let mut all_ping_ms: Vec<f64> = Vec::new();
    let mut all_jitter_ms: Vec<f64> = Vec::new();
    let mut total_dl_bytes: u64 = 0;
    let mut total_ul_bytes: u64 = 0;
    let mut total_dl_duration: f64 = 0.0;
    let mut total_ul_duration: f64 = 0.0;

    for iter in 0..iterations {
        let iter_frac_base = iter as f64 / iterations as f64;
        let iter_frac_step = 1.0 / iterations as f64;

        // ── Download test ────────────────────────────────────────────
        progress(Phase::Ndt7Download, iter_frac_base);

        let dl_result = run_download(&download_url, |frac| {
            progress(
                Phase::Ndt7Download,
                iter_frac_base + frac * iter_frac_step,
            );
        })
        .await?;

        total_dl_bytes += dl_result.bytes;
        total_dl_duration += dl_result.duration_s;

        if dl_result.throughput_mbps > 0.0 {
            all_download_mbps.push(dl_result.throughput_mbps);
        }
        if let Some(p) = dl_result.ping_ms {
            all_ping_ms.push(p);
        }
        if let Some(j) = dl_result.jitter_ms {
            all_jitter_ms.push(j);
        }

        // ── Upload test ──────────────────────────────────────────────
        progress(Phase::Ndt7Upload, iter_frac_base);

        let ul_result = run_upload(&upload_url, |frac| {
            progress(
                Phase::Ndt7Upload,
                iter_frac_base + frac * iter_frac_step,
            );
        })
        .await?;

        total_ul_bytes += ul_result.bytes;
        total_ul_duration += ul_result.duration_s;

        if ul_result.throughput_mbps > 0.0 {
            all_upload_mbps.push(ul_result.throughput_mbps);
        }
        // Collect ping/jitter from upload too (if download didn't yield any)
        if let Some(p) = ul_result.ping_ms {
            all_ping_ms.push(p);
        }
        if let Some(j) = ul_result.jitter_ms {
            all_jitter_ms.push(j);
        }
    }

    progress(Phase::Ndt7Download, 1.0);
    progress(Phase::Ndt7Upload, 1.0);

    // ── Aggregate across iterations ──────────────────────────────────
    let download_mbps = if all_download_mbps.is_empty() {
        None
    } else {
        Some(
            all_download_mbps.iter().sum::<f64>() / all_download_mbps.len() as f64,
        )
    };

    let upload_mbps = if all_upload_mbps.is_empty() {
        None
    } else {
        Some(
            all_upload_mbps.iter().sum::<f64>() / all_upload_mbps.len() as f64,
        )
    };

    let ping_ms = if all_ping_ms.is_empty() {
        None
    } else {
        // Use the minimum ping across all iterations
        all_ping_ms
            .iter()
            .copied()
            .min_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal))
    };

    let jitter_ms = if all_jitter_ms.is_empty() {
        None
    } else {
        Some(
            all_jitter_ms.iter().sum::<f64>() / all_jitter_ms.len() as f64,
        )
    };

    Ok(ProviderResult {
        provider: "M-Lab NDT7".to_string(),
        server: machine,
        location,
        ping_ms,
        jitter_ms,
        download_mbps,
        upload_mbps,
        download_bytes: total_dl_bytes,
        upload_bytes: total_ul_bytes,
        download_duration_s: total_dl_duration,
        upload_duration_s: total_ul_duration,
        packet_loss_pct: None,
        error: None,
    })
}

/// Result from a single download or upload sub-test.
struct SubTestResult {
    throughput_mbps: f64,
    bytes: u64,
    duration_s: f64,
    ping_ms: Option<f64>,
    jitter_ms: Option<f64>,
}

/// Run a single NDT7 download test over WebSocket.
async fn run_download<F>(url: &str, progress: F) -> Result<SubTestResult, String>
where
    F: Fn(f64),
{
    let (ws, _) = tokio_tungstenite::connect_async(url)
        .await
        .map_err(|e| format!("NDT7 download WebSocket connect failed: {e}"))?;

    let (_, mut read) = ws.split();

    let mut min_rtts: Vec<f64> = Vec::new();
    let mut smoothed_rtts: Vec<f64> = Vec::new();
    let mut final_bytes: u64 = 0;
    let mut final_elapsed_us: u64 = 0;
    let mut total_received: u64 = 0;
    let start = Instant::now();

    loop {
        let msg = tokio::select! {
            msg = read.next() => msg,
            _ = tokio::time::sleep(SINGLE_TEST_TIMEOUT) => break,
        };

        match msg {
            Some(Ok(Message::Binary(data))) => {
                total_received += data.len() as u64;
                let elapsed = start.elapsed().as_secs_f64();
                // Assume ~10 seconds per test
                progress((elapsed / 10.0).min(0.99));
            }
            Some(Ok(Message::Text(text))) => {
                if let Ok(measurement) = serde_json::from_str::<serde_json::Value>(&text) {
                    // Extract TCPInfo metrics
                    if let Some(min_rtt) = measurement["TCPInfo"]["MinRTT"].as_u64() {
                        if min_rtt > 0 {
                            min_rtts.push(min_rtt as f64 / 1000.0); // us → ms
                        }
                    }
                    if let Some(smoothed) = measurement["TCPInfo"]["SmoothedRTT"].as_u64() {
                        if smoothed > 0 {
                            smoothed_rtts.push(smoothed as f64 / 1000.0); // us → ms
                        }
                    }

                    // Extract AppInfo metrics
                    if let Some(num_bytes) = measurement["AppInfo"]["NumBytes"].as_u64() {
                        final_bytes = num_bytes;
                    }
                    if let Some(elapsed) = measurement["AppInfo"]["ElapsedTime"].as_u64() {
                        final_elapsed_us = elapsed;
                    }
                }
            }
            Some(Ok(Message::Close(_))) | None => break,
            Some(Err(_)) => break,
            _ => {}
        }
    }

    progress(1.0);

    let duration_s = start.elapsed().as_secs_f64();

    // Throughput from final AppInfo
    let throughput_mbps = if final_elapsed_us > 0 && final_bytes > 0 {
        (final_bytes as f64 * 8.0) / (final_elapsed_us as f64) // bits/us = Mbps
    } else if duration_s > 0.0 && total_received > 0 {
        // Fallback: compute from raw bytes received
        (total_received as f64 * 8.0) / (duration_s * 1_000_000.0)
    } else {
        0.0
    };

    // Ping = minimum MinRTT
    let ping_ms = min_rtts
        .iter()
        .copied()
        .min_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));

    // Jitter = avg abs diff of consecutive SmoothedRTT
    let jitter_ms = avg_consecutive_diff(&smoothed_rtts);

    Ok(SubTestResult {
        throughput_mbps,
        bytes: if final_bytes > 0 { final_bytes } else { total_received },
        duration_s,
        ping_ms,
        jitter_ms,
    })
}

/// Run a single NDT7 upload test over WebSocket.
async fn run_upload<F>(url: &str, progress: F) -> Result<SubTestResult, String>
where
    F: Fn(f64),
{
    let (ws, _) = tokio_tungstenite::connect_async(url)
        .await
        .map_err(|e| format!("NDT7 upload WebSocket connect failed: {e}"))?;

    let (mut write, mut read) = ws.split();

    let upload_data = vec![0u8; UPLOAD_FRAME_SIZE];
    let start = Instant::now();
    let send_deadline = start + Duration::from_secs(10);

    let mut min_rtts: Vec<f64> = Vec::new();
    let mut smoothed_rtts: Vec<f64> = Vec::new();
    let mut final_bytes: u64 = 0;
    let mut final_elapsed_us: u64 = 0;
    let mut bytes_sent: u64 = 0;

    // Send data and read server measurements concurrently
    loop {
        let now = Instant::now();
        if now >= send_deadline {
            break;
        }

        tokio::select! {
            // Try to send a frame
            send_result = write.send(Message::Binary(upload_data.clone())) => {
                match send_result {
                    Ok(()) => {
                        bytes_sent += UPLOAD_FRAME_SIZE as u64;
                        let elapsed = start.elapsed().as_secs_f64();
                        progress((elapsed / 10.0).min(0.99));
                    }
                    Err(_) => break,
                }
            }
            // Also read any server measurement messages
            msg = read.next() => {
                match msg {
                    Some(Ok(Message::Text(text))) => {
                        if let Ok(measurement) = serde_json::from_str::<serde_json::Value>(&text) {
                            if let Some(min_rtt) = measurement["TCPInfo"]["MinRTT"].as_u64() {
                                if min_rtt > 0 {
                                    min_rtts.push(min_rtt as f64 / 1000.0);
                                }
                            }
                            if let Some(smoothed) = measurement["TCPInfo"]["SmoothedRTT"].as_u64() {
                                if smoothed > 0 {
                                    smoothed_rtts.push(smoothed as f64 / 1000.0);
                                }
                            }
                            if let Some(num_bytes) = measurement["AppInfo"]["NumBytes"].as_u64() {
                                final_bytes = num_bytes;
                            }
                            if let Some(elapsed) = measurement["AppInfo"]["ElapsedTime"].as_u64() {
                                final_elapsed_us = elapsed;
                            }
                        }
                    }
                    Some(Ok(Message::Close(_))) | None => break,
                    Some(Err(_)) => break,
                    _ => {}
                }
            }
        }
    }

    // Close the write side and drain remaining server measurements
    let _ = write.send(Message::Close(None)).await;

    let drain_deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let msg = tokio::select! {
            msg = read.next() => msg,
            _ = tokio::time::sleep_until(tokio::time::Instant::from_std(drain_deadline)) => break,
        };

        match msg {
            Some(Ok(Message::Text(text))) => {
                if let Ok(measurement) = serde_json::from_str::<serde_json::Value>(&text) {
                    if let Some(min_rtt) = measurement["TCPInfo"]["MinRTT"].as_u64() {
                        if min_rtt > 0 {
                            min_rtts.push(min_rtt as f64 / 1000.0);
                        }
                    }
                    if let Some(smoothed) = measurement["TCPInfo"]["SmoothedRTT"].as_u64() {
                        if smoothed > 0 {
                            smoothed_rtts.push(smoothed as f64 / 1000.0);
                        }
                    }
                    if let Some(num_bytes) = measurement["AppInfo"]["NumBytes"].as_u64() {
                        final_bytes = num_bytes;
                    }
                    if let Some(elapsed) = measurement["AppInfo"]["ElapsedTime"].as_u64() {
                        final_elapsed_us = elapsed;
                    }
                }
            }
            Some(Ok(Message::Close(_))) | None => break,
            Some(Err(_)) => break,
            _ => {}
        }
    }

    progress(1.0);

    let duration_s = start.elapsed().as_secs_f64();

    // Throughput from final server-reported AppInfo
    let throughput_mbps = if final_elapsed_us > 0 && final_bytes > 0 {
        (final_bytes as f64 * 8.0) / (final_elapsed_us as f64)
    } else if duration_s > 0.0 && bytes_sent > 0 {
        (bytes_sent as f64 * 8.0) / (duration_s * 1_000_000.0)
    } else {
        0.0
    };

    let ping_ms = min_rtts
        .iter()
        .copied()
        .min_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));

    let jitter_ms = avg_consecutive_diff(&smoothed_rtts);

    Ok(SubTestResult {
        throughput_mbps,
        bytes: if final_bytes > 0 { final_bytes } else { bytes_sent },
        duration_s,
        ping_ms,
        jitter_ms,
    })
}

/// Average absolute difference between consecutive samples.
/// Returns `None` if fewer than 2 samples.
fn avg_consecutive_diff(values: &[f64]) -> Option<f64> {
    if values.len() < 2 {
        return None;
    }
    let sum: f64 = values
        .windows(2)
        .map(|w| (w[1] - w[0]).abs())
        .sum();
    Some(sum / (values.len() - 1) as f64)
}

/// Build an error ProviderResult with zeroed metrics.
fn error_result(msg: String) -> ProviderResult {
    ProviderResult {
        provider: "M-Lab NDT7".to_string(),
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
    }
}
