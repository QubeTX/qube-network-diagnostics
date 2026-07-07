//! M-Lab MSAK (throughput1) provider — the multi-stream successor to NDT7.
//!
//! NDT7 deliberately measures single-TCP-flow bulk transport capacity, which
//! structurally underestimates on high-BDP or lossy paths. MSAK runs multiple
//! parallel streams and measures aggregate capacity, the way Ookla-class
//! tests do — a genuinely different signal even against the same M-Lab fleet.
//!
//! Protocol (m-lab/msak, Apache-2.0; same open platform and policy as the
//! NDT7 integration): locate via
//! `locate.measurementlab.net/v2/nearest/msak/throughput1`, WebSocket
//! subprotocol `net.measurementlab.throughput.v1`, binary frames carry data,
//! JSON text frames carry `WireMeasurement` ({Application,Network}.{BytesSent,
//! BytesReceived}, ElapsedTime in µs, optional TCPInfo with MinRTT/RTT in µs).
//! `streams` is a required query param; `duration` is in milliseconds, capped
//! at 25000 by the server.

use super::ndt7::{should_grow_frame, INITIAL_UPLOAD_FRAME_SIZE};
use super::{
    statistics, BandwidthSamples, Phase, ProviderAvailability, ProviderResult, SpeedTestConfig,
    TestDuration,
};
use futures_util::{SinkExt, StreamExt};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::Message;

const LOCATE_URL: &str = "https://locate.measurementlab.net/v2/nearest/msak/throughput1";

const SUBPROTOCOL: &str = "net.measurementlab.throughput.v1";

/// Parallel streams per direction (the reference client default).
const STREAMS: usize = 2;

/// Server-side cap on a single subtest (milliseconds).
const MAX_SERVER_DURATION_MS: u64 = 25_000;

/// Aggregate sampling tick across both streams.
const SAMPLE_INTERVAL: Duration = Duration::from_millis(500);

/// Run the M-Lab MSAK multi-stream speed test.
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
    // ── Server discovery (same Locate API envelope as NDT7) ─────────
    progress(Phase::MsakDiscovery, 0.0);

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(15))
        .build()
        .map_err(|e| format!("HTTP client error: {e}"))?;

    let resp = client
        .get(LOCATE_URL)
        .send()
        .await
        .map_err(|e| format!("MSAK discovery failed: {e}"))?;

    // Locate refusals (429 in particular) would otherwise surface as a
    // misleading "missing results array" shape error — name the condition.
    let status = resp.status();
    if status.as_u16() == 429 {
        return Err(
            "MSAK discovery rate-limited: M-Lab Locate refused this network (too many requests) — try again later"
                .to_string(),
        );
    }
    if !status.is_success() {
        return Err(format!("MSAK discovery failed: HTTP {}", status.as_u16()));
    }

    let body: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| format!("MSAK discovery parse error: {e}"))?;

    let results = body["results"]
        .as_array()
        .ok_or("MSAK discovery: missing results array")?;

    let server_entry = results.first().ok_or("MSAK discovery: no servers found")?;

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

    // ── Duration budget (per direction) ─────────────────────────────
    let budget_secs = match &config.duration {
        TestDuration::Seconds(s) => *s,
        TestDuration::Auto => 10,
    };
    let session_ms = (budget_secs * 1000).min(MAX_SERVER_DURATION_MS);

    let download_url = build_session_url(
        urls["wss:///throughput/v1/download"]
            .as_str()
            .ok_or("MSAK discovery: missing download URL")?,
        session_ms,
    )?;
    let upload_url = build_session_url(
        urls["wss:///throughput/v1/upload"]
            .as_str()
            .ok_or("MSAK discovery: missing upload URL")?,
        session_ms,
    )?;
    let download_url_ws = urls["ws:///throughput/v1/download"]
        .as_str()
        .and_then(|u| build_session_url(u, session_ms).ok());
    let upload_url_ws = urls["ws:///throughput/v1/upload"]
        .as_str()
        .and_then(|u| build_session_url(u, session_ms).ok());

    progress(Phase::MsakDiscovery, 1.0);

    // ── Download ─────────────────────────────────────────────────────
    progress(Phase::MsakDownload, 0.0);
    let dl = transfer(
        Direction::Download,
        &download_url,
        download_url_ws.as_deref(),
        Duration::from_secs(budget_secs),
        |frac| progress(Phase::MsakDownload, frac),
    )
    .await;
    progress(Phase::MsakDownload, 1.0);

    // ── Upload ───────────────────────────────────────────────────────
    progress(Phase::MsakUpload, 0.0);
    let ul = transfer(
        Direction::Upload,
        &upload_url,
        upload_url_ws.as_deref(),
        Duration::from_secs(budget_secs),
        |frac| progress(Phase::MsakUpload, frac),
    )
    .await;
    progress(Phase::MsakUpload, 1.0);

    if dl.samples.is_empty() && ul.samples.is_empty() {
        return Err("no successful transfers".to_string());
    }

    let download_mbps = if dl.samples.is_empty() {
        None
    } else {
        Some(statistics::accurate_bandwidth(&dl.samples))
    };
    let upload_mbps = if ul.samples.is_empty() {
        None
    } else {
        Some(statistics::accurate_upload_bandwidth(&ul.samples))
    };

    // Ping = minimum kernel MinRTT seen on any stream in either direction.
    let ping_ms = dl
        .min_rtts
        .iter()
        .chain(ul.min_rtts.iter())
        .copied()
        .min_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));

    let all_rtts: Vec<f64> = dl.rtts.iter().chain(ul.rtts.iter()).copied().collect();
    let jitter_ms = if all_rtts.len() >= 2 {
        Some(statistics::jitter_rfc3550(&all_rtts))
    } else {
        None
    };

    Ok(ProviderResult {
        provider: "M-Lab MSAK".to_string(),
        server: machine,
        location,
        ping_ms,
        jitter_ms,
        download_mbps,
        upload_mbps,
        download_bytes: dl.bytes,
        upload_bytes: ul.bytes,
        download_duration_s: dl.duration_s,
        upload_duration_s: ul.duration_s,
        packet_loss_pct: None,
        error: None,
        bandwidth_samples: Some(BandwidthSamples {
            download: dl.samples,
            upload: ul.samples,
        }),
        availability: ProviderAvailability::Ran,
        latency_stats: None,
        loaded_latency: None,
    })
}

/// Append the required/etiquette query params to a Locate URL (which already
/// carries a signed `access_token`).
fn build_session_url(base: &str, duration_ms: u64) -> Result<String, String> {
    let mut url = url::Url::parse(base).map_err(|e| format!("MSAK URL parse error: {e}"))?;
    url.query_pairs_mut()
        .append_pair("streams", &STREAMS.to_string())
        .append_pair("duration", &duration_ms.to_string())
        .append_pair("client_name", "speedqx")
        .append_pair("client_version", env!("CARGO_PKG_VERSION"))
        .append_pair("client_library_name", "nd300-msak")
        .append_pair("client_library_version", env!("CARGO_PKG_VERSION"));
    Ok(url.into())
}

#[derive(Clone, Copy, PartialEq)]
enum Direction {
    Download,
    Upload,
}

/// Aggregate outcome of one direction (both streams).
struct TransferOutcome {
    /// 500ms aggregate throughput samples across both streams.
    samples: Vec<f64>,
    bytes: u64,
    duration_s: f64,
    min_rtts: Vec<f64>,
    rtts: Vec<f64>,
}

/// Per-stream RTT collections returned by a stream task.
#[derive(Default)]
struct StreamStats {
    min_rtts: Vec<f64>,
    rtts: Vec<f64>,
}

/// Run one direction: spawn [`STREAMS`] WebSocket streams that update shared
/// byte counters, while this task samples the aggregate counter every 500ms.
async fn transfer<F>(
    direction: Direction,
    url: &str,
    fallback_url: Option<&str>,
    budget: Duration,
    progress: F,
) -> TransferOutcome
where
    F: Fn(f64),
{
    let start = Instant::now();
    let deadline = tokio::time::Instant::from_std(start + budget + Duration::from_secs(5));

    // Per-stream byte counters. Download: client-received binary bytes
    // (client side IS ground truth). Upload: latest server-reported
    // Application.BytesReceived (receiver-side truth — client `bytes_sent`
    // only measures buffer fill).
    let counters: Vec<Arc<AtomicU64>> = (0..STREAMS).map(|_| Arc::new(AtomicU64::new(0))).collect();

    let mut handles = Vec::new();
    for counter in counters.iter() {
        let url = url.to_string();
        let fallback = fallback_url.map(|s| s.to_string());
        let counter = counter.clone();
        handles.push(tokio::spawn(async move {
            match direction {
                Direction::Download => download_stream(&url, fallback.as_deref(), counter).await,
                Direction::Upload => upload_stream(&url, fallback.as_deref(), counter).await,
            }
        }));
    }

    // Sample the aggregate counter every 500ms until every stream finishes
    // or the deadline passes.
    let mut samples: Vec<f64> = Vec::new();
    let mut last_total: u64 = 0;
    let mut last_at = Instant::now();
    let mut sampler = tokio::time::interval(SAMPLE_INTERVAL);
    sampler.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    sampler.reset();

    let mut joined = futures_util::future::join_all(handles);
    let stream_stats: Vec<StreamStats> = loop {
        tokio::select! {
            results = &mut joined => {
                break results
                    .into_iter()
                    .map(|r| r.unwrap_or_default())
                    .collect();
            }
            _ = sampler.tick() => {
                let total: u64 = counters.iter().map(|c| c.load(Ordering::Relaxed)).sum();
                let now = Instant::now();
                let dt = now.duration_since(last_at).as_secs_f64();
                let db = total.saturating_sub(last_total);
                if dt > 0.1 && db > 0 {
                    samples.push(db as f64 * 8.0 / (dt * 1_000_000.0));
                }
                last_total = total;
                last_at = now;
                progress((start.elapsed().as_secs_f64() / budget.as_secs_f64()).min(0.99));
            }
            _ = tokio::time::sleep_until(deadline) => {
                // Streams wedged past the budget — abandon them; the spawned
                // tasks are detached and bounded by their own read deadlines.
                break Vec::new();
            }
        }
    };

    let mut min_rtts = Vec::new();
    let mut rtts = Vec::new();
    for s in stream_stats {
        min_rtts.extend(s.min_rtts);
        rtts.extend(s.rtts);
    }

    TransferOutcome {
        samples,
        bytes: counters.iter().map(|c| c.load(Ordering::Relaxed)).sum(),
        duration_s: start.elapsed().as_secs_f64(),
        min_rtts,
        rtts,
    }
}

/// Connect with the throughput1 subprotocol; wss → ws fallback like NDT7.
async fn msak_connect(
    url: &str,
    fallback_url: Option<&str>,
) -> Result<
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>,
    String,
> {
    let build_req = |u: &str| -> Result<_, String> {
        let mut req = u
            .into_client_request()
            .map_err(|e| format!("MSAK request build failed: {e}"))?;
        req.headers_mut().insert(
            "Sec-WebSocket-Protocol",
            SUBPROTOCOL
                .parse()
                .map_err(|e| format!("MSAK header error: {e}"))?,
        );
        Ok(req)
    };

    match tokio_tungstenite::connect_async(build_req(url)?).await {
        Ok((ws, _)) => Ok(ws),
        Err(e) => {
            if let Some(ws_url) = fallback_url {
                tokio_tungstenite::connect_async(build_req(ws_url)?)
                    .await
                    .map(|(ws, _)| ws)
                    .map_err(|e2| format!("MSAK WebSocket connect failed: wss: {e}, ws: {e2}"))
            } else {
                Err(format!("MSAK WebSocket connect failed: {e}"))
            }
        }
    }
}

/// Read binary frames, adding byte counts to the shared counter; harvest
/// TCPInfo RTTs from text measurements. The server closes the connection
/// when the session `duration` elapses.
async fn download_stream(
    url: &str,
    fallback_url: Option<&str>,
    counter: Arc<AtomicU64>,
) -> StreamStats {
    let mut stats = StreamStats::default();
    let Ok(ws) = msak_connect(url, fallback_url).await else {
        return stats;
    };
    let (_, mut read) = ws.split();

    // Hard cap: server max duration plus headroom.
    let hard_cap =
        tokio::time::Instant::now() + Duration::from_millis(MAX_SERVER_DURATION_MS + 10_000);

    loop {
        let msg = tokio::select! {
            msg = read.next() => msg,
            _ = tokio::time::sleep_until(hard_cap) => break,
        };
        match msg {
            Some(Ok(Message::Binary(data))) => {
                counter.fetch_add(data.len() as u64, Ordering::Relaxed);
            }
            Some(Ok(Message::Text(text))) => ingest_rtts(&text, &mut stats),
            Some(Ok(Message::Close(_))) | None => break,
            Some(Err(_)) => break,
            _ => {}
        }
    }
    stats
}

/// Send growing binary frames; the shared counter tracks the latest
/// server-reported received-byte count from this stream's measurements.
async fn upload_stream(
    url: &str,
    fallback_url: Option<&str>,
    counter: Arc<AtomicU64>,
) -> StreamStats {
    let mut stats = StreamStats::default();
    let Ok(ws) = msak_connect(url, fallback_url).await else {
        return stats;
    };
    let (mut write, mut read) = ws.split();

    let mut frame_size = INITIAL_UPLOAD_FRAME_SIZE;
    let mut upload_data = vec![0u8; frame_size];
    let mut bytes_sent: u64 = 0;
    let hard_cap =
        tokio::time::Instant::now() + Duration::from_millis(MAX_SERVER_DURATION_MS + 10_000);

    loop {
        tokio::select! {
            _ = tokio::time::sleep_until(hard_cap) => break,
            send_result = write.send(Message::Binary(upload_data.clone())) => {
                match send_result {
                    Ok(()) => {
                        bytes_sent += frame_size as u64;
                        if should_grow_frame(frame_size, bytes_sent) {
                            frame_size = (frame_size * 2).min(super::ndt7::MAX_UPLOAD_FRAME_SIZE);
                            upload_data = vec![0u8; frame_size];
                        }
                    }
                    Err(_) => break,
                }
            }
            msg = read.next() => {
                match msg {
                    Some(Ok(Message::Text(text))) => {
                        ingest_rtts(&text, &mut stats);
                        if let Some(received) = parse_app_bytes_received(&text) {
                            counter.store(received, Ordering::Relaxed);
                        }
                    }
                    Some(Ok(Message::Close(_))) | None => break,
                    Some(Err(_)) => break,
                    _ => {}
                }
            }
        }
    }

    let _ = write.send(Message::Close(None)).await;
    stats
}

/// Harvest TCPInfo MinRTT/RTT (µs → ms) from a WireMeasurement text frame.
fn ingest_rtts(text: &str, stats: &mut StreamStats) {
    if let Ok(measurement) = serde_json::from_str::<serde_json::Value>(text) {
        if let Some(min_rtt) = measurement["TCPInfo"]["MinRTT"].as_u64() {
            if min_rtt > 0 {
                stats.min_rtts.push(min_rtt as f64 / 1000.0);
            }
        }
        if let Some(rtt) = measurement["TCPInfo"]["RTT"].as_u64() {
            if rtt > 0 {
                stats.rtts.push(rtt as f64 / 1000.0);
            }
        }
    }
}

/// The server-side received-byte count from a WireMeasurement text frame.
fn parse_app_bytes_received(text: &str) -> Option<u64> {
    serde_json::from_str::<serde_json::Value>(text)
        .ok()?
        .pointer("/Application/BytesReceived")?
        .as_u64()
}

pub(crate) fn error_result(msg: String) -> ProviderResult {
    ProviderResult {
        provider: "M-Lab MSAK".to_string(),
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

    /// A WireMeasurement fixture matching m-lab/msak's
    /// pkg/throughput1/model/measurement.go shape.
    const WIRE_MEASUREMENT: &str = r#"{
        "CC": "bbr",
        "UUID": "test-uuid",
        "LocalAddr": "10.0.0.1:443",
        "RemoteAddr": "203.0.113.5:51234",
        "Application": {"BytesSent": 0, "BytesReceived": 1048576},
        "Network": {"BytesSent": 0, "BytesReceived": 1100000},
        "ElapsedTime": 500000,
        "TCPInfo": {"MinRTT": 12345, "RTT": 15678}
    }"#;

    #[test]
    fn wire_measurement_rtts_convert_us_to_ms() {
        let mut stats = StreamStats::default();
        ingest_rtts(WIRE_MEASUREMENT, &mut stats);
        assert_eq!(stats.min_rtts, vec![12.345]);
        assert_eq!(stats.rtts, vec![15.678]);
    }

    #[test]
    fn wire_measurement_app_bytes_received_parsed() {
        assert_eq!(parse_app_bytes_received(WIRE_MEASUREMENT), Some(1_048_576));
        assert_eq!(parse_app_bytes_received("{}"), None);
        assert_eq!(parse_app_bytes_received("not json"), None);
    }

    /// Params must append correctly to a Locate URL that already carries a
    /// signed access_token query string.
    #[test]
    fn session_url_appends_params_to_signed_locate_url() {
        let base = "wss://mlab1-abc.mlab-oti.measurement-lab.org/throughput/v1/download?access_token=abc123";
        let url = build_session_url(base, 10_000).unwrap();
        assert!(url.contains("access_token=abc123"), "url: {url}");
        assert!(url.contains("streams=2"), "url: {url}");
        assert!(url.contains("duration=10000"), "url: {url}");
        assert!(url.contains("client_name=speedqx"), "url: {url}");
    }

    #[test]
    fn session_url_rejects_invalid_base() {
        assert!(build_session_url("not a url", 10_000).is_err());
    }
}
