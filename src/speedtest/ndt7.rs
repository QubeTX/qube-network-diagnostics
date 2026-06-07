use super::{statistics, BandwidthSamples, Phase, ProviderResult, SpeedTestConfig, TestDuration};
use futures_util::{SinkExt, StreamExt};
use std::time::{Duration, Instant};
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::Message;

const LOCATE_URL: &str = "https://locate.measurementlab.net/v2/nearest/ndt/ndt7";

/// Initial upload frame size (8 KB) — doubles up to MAX_UPLOAD_FRAME.
const INITIAL_UPLOAD_FRAME_SIZE: usize = 8192;

/// Maximum upload frame size (1 MB).
const MAX_UPLOAD_FRAME_SIZE: usize = 1 << 20;

/// Minimum remaining time to start a new iteration (3 seconds).
const MIN_REMAINING_SECS: u64 = 3;

/// Maximum duration to wait for a single download or upload test (safety cap).
const SINGLE_TEST_TIMEOUT: Duration = Duration::from_secs(30);

/// Run the NDT7 (M-Lab) speed test: server discovery, download, upload.
pub async fn run<F>(config: &SpeedTestConfig, progress: F) -> ProviderResult
where
    F: Fn(Phase, f64) + Send + Sync,
{
    match run_inner(config, &progress).await {
        Ok(result) => result,
        Err(e) => error_result(e.to_string()),
    }
}

/// Internal implementation that propagates errors.
async fn run_inner<F>(config: &SpeedTestConfig, progress: &F) -> Result<ProviderResult, String>
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

    let server_entry = results.first().ok_or("NDT7 discovery: no servers found")?;

    let machine = server_entry["machine"]
        .as_str()
        .unwrap_or("unknown")
        .to_string();

    let city = server_entry["location"]["city"].as_str().unwrap_or("");
    let country = server_entry["location"]["country"].as_str().unwrap_or("");
    let location = if !city.is_empty() || !country.is_empty() {
        Some(format!(
            "{}{}{}",
            city,
            if !city.is_empty() && !country.is_empty() {
                ", "
            } else {
                ""
            },
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

    // Fallback plain WS URLs (no TLS) — used if WSS connection fails
    let download_url_ws = urls["ws:///ndt/v7/download"]
        .as_str()
        .map(|s| s.to_string());
    let upload_url_ws = urls["ws:///ndt/v7/upload"].as_str().map(|s| s.to_string());

    progress(Phase::Ndt7Discovery, 1.0);

    // ── Duration budget (per direction) ─────────────────────────────
    let (dl_budget_secs, ul_budget_secs) = match &config.duration {
        TestDuration::Seconds(s) => (*s, *s),
        TestDuration::Auto => (10, 10),
    };

    let mut all_download_mbps: Vec<f64> = Vec::new();
    let mut all_upload_mbps: Vec<f64> = Vec::new();
    let mut all_ping_ms: Vec<f64> = Vec::new();
    let mut all_smoothed_rtts: Vec<f64> = Vec::new();
    let mut total_dl_bytes: u64 = 0;
    let mut total_ul_bytes: u64 = 0;
    let mut total_dl_duration: f64 = 0.0;
    let mut total_ul_duration: f64 = 0.0;

    // ── Download: deadline-based loop ────────────────────────────────
    let dl_phase_start = Instant::now();
    let dl_deadline = dl_phase_start + Duration::from_secs(dl_budget_secs);

    progress(Phase::Ndt7Download, 0.0);

    loop {
        let remaining = dl_deadline.saturating_duration_since(Instant::now());
        if remaining < Duration::from_secs(MIN_REMAINING_SECS) {
            break;
        }

        let run_duration = remaining.min(Duration::from_secs(10));
        let dl_budget_f64 = dl_budget_secs as f64;
        let dl_result = run_download(
            &download_url,
            download_url_ws.as_deref(),
            run_duration,
            |frac| {
                // Progress based on overall deadline, not per-session
                let elapsed = dl_phase_start.elapsed().as_secs_f64();
                let overall_frac = (elapsed / dl_budget_f64).min(0.99);
                // Blend: use max of overall progress and per-session fraction
                progress(Phase::Ndt7Download, overall_frac.max(frac * 0.1));
            },
        )
        .await?;

        total_dl_bytes += dl_result.bytes;
        total_dl_duration += dl_result.duration_s;

        if dl_result.throughput_mbps > 0.0 {
            all_download_mbps.push(dl_result.throughput_mbps);
        }
        if let Some(p) = dl_result.ping_ms {
            all_ping_ms.push(p);
        }
        all_smoothed_rtts.extend_from_slice(&dl_result.smoothed_rtts);

        // Update progress based on elapsed time
        let elapsed = dl_phase_start.elapsed().as_secs_f64();
        progress(Phase::Ndt7Download, (elapsed / dl_budget_f64).min(0.99));
    }

    progress(Phase::Ndt7Download, 1.0);

    // ── Upload: deadline-based loop ──────────────────────────────────
    let ul_phase_start = Instant::now();
    let ul_deadline = ul_phase_start + Duration::from_secs(ul_budget_secs);

    progress(Phase::Ndt7Upload, 0.0);

    loop {
        let remaining = ul_deadline.saturating_duration_since(Instant::now());
        if remaining < Duration::from_secs(MIN_REMAINING_SECS) {
            break;
        }

        let run_duration = remaining.min(Duration::from_secs(10));
        let ul_budget_f64 = ul_budget_secs as f64;
        let ul_result = run_upload(
            &upload_url,
            upload_url_ws.as_deref(),
            run_duration,
            |frac| {
                let elapsed = ul_phase_start.elapsed().as_secs_f64();
                let overall_frac = (elapsed / ul_budget_f64).min(0.99);
                progress(Phase::Ndt7Upload, overall_frac.max(frac * 0.1));
            },
        )
        .await?;

        total_ul_bytes += ul_result.bytes;
        total_ul_duration += ul_result.duration_s;

        if ul_result.throughput_mbps > 0.0 {
            all_upload_mbps.push(ul_result.throughput_mbps);
        }
        if let Some(p) = ul_result.ping_ms {
            all_ping_ms.push(p);
        }
        all_smoothed_rtts.extend_from_slice(&ul_result.smoothed_rtts);

        let elapsed = ul_phase_start.elapsed().as_secs_f64();
        progress(Phase::Ndt7Upload, (elapsed / ul_budget_f64).min(0.99));
    }

    progress(Phase::Ndt7Upload, 1.0);

    // ── Aggregate across iterations using statistics pipeline ───────
    let download_mbps = if all_download_mbps.is_empty() {
        None
    } else {
        Some(statistics::accurate_bandwidth(&all_download_mbps))
    };

    let upload_mbps = if all_upload_mbps.is_empty() {
        None
    } else {
        Some(statistics::accurate_upload_bandwidth(&all_upload_mbps))
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

    // RFC 3550 jitter from all smoothed RTT samples
    let jitter_ms = if all_smoothed_rtts.len() >= 2 {
        Some(statistics::jitter_rfc3550(&all_smoothed_rtts))
    } else {
        None
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
        bandwidth_samples: Some(BandwidthSamples {
            download: all_download_mbps,
            upload: all_upload_mbps,
        }),
    })
}

/// Result from a single download or upload sub-test.
struct SubTestResult {
    throughput_mbps: f64,
    bytes: u64,
    duration_s: f64,
    ping_ms: Option<f64>,
    smoothed_rtts: Vec<f64>,
}

/// Connect to an NDT7 WebSocket endpoint with the required subprotocol header.
/// Tries the primary URL first; falls back to the alternate URL on failure.
async fn ndt7_connect(
    url: &str,
    fallback_url: Option<&str>,
    label: &str,
) -> Result<
    (
        tokio_tungstenite::WebSocketStream<
            tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
        >,
        tokio_tungstenite::tungstenite::http::Response<Option<Vec<u8>>>,
    ),
    String,
> {
    let mut req = url
        .into_client_request()
        .map_err(|e| format!("NDT7 {label} request build failed: {e}"))?;
    req.headers_mut().insert(
        "Sec-WebSocket-Protocol",
        "net.measurementlab.ndt.v7"
            .parse()
            .expect("valid header value"),
    );

    match tokio_tungstenite::connect_async(req).await {
        Ok(conn) => Ok(conn),
        Err(e) => {
            if let Some(ws_url) = fallback_url {
                let mut fallback_req = ws_url
                    .into_client_request()
                    .map_err(|e| format!("NDT7 {label} fallback request build failed: {e}"))?;
                fallback_req.headers_mut().insert(
                    "Sec-WebSocket-Protocol",
                    "net.measurementlab.ndt.v7"
                        .parse()
                        .expect("valid header value"),
                );
                tokio_tungstenite::connect_async(fallback_req)
                    .await
                    .map_err(|e2| {
                        format!("NDT7 {label} WebSocket connect failed: wss: {e}, ws: {e2}")
                    })
            } else {
                Err(format!("NDT7 {label} WebSocket connect failed: {e}"))
            }
        }
    }
}

/// Run a single NDT7 download test over WebSocket.
async fn run_download<F>(
    url: &str,
    fallback_url: Option<&str>,
    duration: Duration,
    progress: F,
) -> Result<SubTestResult, String>
where
    F: Fn(f64),
{
    let (ws, _) = ndt7_connect(url, fallback_url, "download").await?;

    let (_, mut read) = ws.split();

    let mut min_rtts: Vec<f64> = Vec::new();
    let mut smoothed_rtts: Vec<f64> = Vec::new();
    let mut final_bytes: u64 = 0;
    let mut final_elapsed_us: u64 = 0;
    let mut total_received: u64 = 0;
    let start = Instant::now();
    let deadline = tokio::time::Instant::from_std(start + duration);
    // Pinned absolute safety cap computed ONCE: a single fixed instant the
    // select! fires at no matter how many loop iterations run. (The previous
    // `sleep(SINGLE_TEST_TIMEOUT)` re-armed a fresh 30s timer every iteration,
    // so it could never actually fire and was dead.)
    let hard_cap = tokio::time::Instant::from_std(start + SINGLE_TEST_TIMEOUT);

    loop {
        let msg = tokio::select! {
            msg = read.next() => msg,
            _ = tokio::time::sleep_until(deadline) => break,
            _ = tokio::time::sleep_until(hard_cap) => break,
        };

        match msg {
            Some(Ok(Message::Binary(data))) => {
                total_received += data.len() as u64;
                let elapsed = start.elapsed().as_secs_f64();
                progress((elapsed / duration.as_secs_f64()).min(0.99));
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

    Ok(SubTestResult {
        throughput_mbps,
        bytes: if final_bytes > 0 {
            final_bytes
        } else {
            total_received
        },
        duration_s,
        ping_ms,
        smoothed_rtts,
    })
}

/// Run a single NDT7 upload test over WebSocket.
async fn run_upload<F>(
    url: &str,
    fallback_url: Option<&str>,
    duration: Duration,
    progress: F,
) -> Result<SubTestResult, String>
where
    F: Fn(f64),
{
    let (ws, _) = ndt7_connect(url, fallback_url, "upload").await?;

    let (mut write, mut read) = ws.split();

    let mut frame_size = INITIAL_UPLOAD_FRAME_SIZE;
    let mut upload_data = vec![0u8; frame_size];
    let start = Instant::now();
    let send_deadline = start + duration;
    // Pinned absolute safety cap computed ONCE: bounds the whole upload loop so
    // a `write.send` that wedges (e.g. a half-open socket that never flushes)
    // can't hang past SINGLE_TEST_TIMEOUT. Mirrors the download loop's cap.
    let hard_cap = tokio::time::Instant::from_std(start + SINGLE_TEST_TIMEOUT);
    let mut frame_count: u64 = 0;

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
            // Absolute safety cap: fires once at a fixed instant regardless of
            // how the send/read arms behave, so a stuck send can't hang here.
            _ = tokio::time::sleep_until(hard_cap) => break,
            // Try to send a frame
            send_result = write.send(Message::Binary(upload_data.clone())) => {
                match send_result {
                    Ok(()) => {
                        bytes_sent += frame_size as u64;
                        frame_count += 1;
                        let elapsed = start.elapsed().as_secs_f64();
                        progress((elapsed / duration.as_secs_f64()).min(0.99));

                        // Double frame size periodically for better saturation
                        if frame_count.is_multiple_of(100) && frame_size < MAX_UPLOAD_FRAME_SIZE {
                            frame_size *= 2;
                            upload_data = vec![0u8; frame_size];
                        }
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

    Ok(SubTestResult {
        throughput_mbps,
        bytes: if final_bytes > 0 {
            final_bytes
        } else {
            bytes_sent
        },
        duration_s,
        ping_ms,
        smoothed_rtts,
    })
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
        bandwidth_samples: None,
    }
}
