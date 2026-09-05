//! Receiver-accounted acquisition. All transfer futures are owned by the current
//! direction; dropping one cancels its HTTP requests and WebSockets.
use super::measurement_v5::{CounterPoint, MeasurementTrace, WARMUP_MS};
use futures_util::{future::join_all, SinkExt, StreamExt};
use rand::RngCore;
use std::sync::{
    atomic::{AtomicBool, AtomicU64, Ordering},
    Arc, Mutex,
};
use std::time::{Duration, Instant};
use tokio_tungstenite::tungstenite::{client::IntoClientRequest, Message};

struct Accounting {
    used: u64,
    reserved: u64,
    reason: String,
}
pub struct RunBudget {
    pub started: Instant,
    pub limit: u64,
    cap: Duration,
    cancel: Arc<AtomicBool>,
    state: Mutex<Accounting>,
}
impl RunBudget {
    pub fn stop(&self, reason: &str) {
        let mut state = self.state.lock().unwrap();
        if state.reason == "complete" {
            state.reason = reason.into();
        }
    }
    pub fn new(limit: u64, cap: Duration, cancel: Arc<AtomicBool>) -> Self {
        Self {
            started: Instant::now(),
            limit,
            cap,
            cancel,
            state: Mutex::new(Accounting {
                used: 0,
                reserved: 0,
                reason: "complete".into(),
            }),
        }
    }
    pub fn stopped(&self) -> bool {
        let mut state = self.state.lock().unwrap();
        if state.reason == "complete" {
            if self.cancel.load(Ordering::Relaxed) {
                state.reason = "cancelled".into();
            } else if self.started.elapsed() >= self.cap {
                state.reason = "time-limit".into();
            }
        }
        state.reason != "complete"
    }
    pub fn reason(&self) -> String {
        self.stopped();
        self.state.lock().unwrap().reason.clone()
    }
    pub fn used(&self) -> u64 {
        self.state.lock().unwrap().used
    }
    pub async fn wait_stopped(&self) {
        while !self.stopped() {
            tokio::time::sleep(
                Duration::from_millis(25).min(self.cap.saturating_sub(self.started.elapsed())),
            )
            .await;
        }
    }
    fn reserve(&self, wanted: u64) -> u64 {
        if self.stopped() {
            return 0;
        }
        let mut s = self.state.lock().unwrap();
        let n = wanted.min(self.limit.saturating_sub(s.used).saturating_sub(s.reserved));
        s.reserved += n;
        if n == 0 && s.reserved == 0 {
            s.reason = "byte-limit".into();
        }
        n
    }
    fn release(&self, n: u64) {
        let mut s = self.state.lock().unwrap();
        s.reserved = s.reserved.saturating_sub(n);
    }
    fn consume(&self, n: u64, reservation: u64) {
        let mut s = self.state.lock().unwrap();
        s.reserved = s.reserved.saturating_sub(reservation);
        s.used += n;
        if s.used >= self.limit && s.reason == "complete" {
            s.reason = "byte-limit".into();
        }
    }
}

struct Reservation<'a> {
    budget: &'a RunBudget,
    remaining: u64,
}
impl Drop for Reservation<'_> {
    fn drop(&mut self) {
        self.budget.release(self.remaining);
    }
}

pub fn client() -> Result<reqwest::Client, reqwest::Error> {
    reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(8))
        .timeout(Duration::from_secs(30))
        .pool_max_idle_per_host(2)
        .http2_adaptive_window(true)
        .no_gzip()
        .no_brotli()
        .no_deflate()
        .no_zstd()
        .build()
}

fn synthetic_payload(size: usize) -> Vec<u8> {
    let mut bytes = vec![0; size];
    rand::thread_rng().fill_bytes(&mut bytes);
    bytes
}

pub async fn locate(
    provider: &str,
    duration_ms: u64,
    client: &reqwest::Client,
) -> Result<(String, String, String), String> {
    let path = if provider == "msak" {
        "msak/throughput1"
    } else {
        "ndt/ndt7"
    };
    let response = client
        .get(format!(
            "https://locate.measurementlab.net/v2/nearest/{path}"
        ))
        .timeout(Duration::from_secs(8))
        .send()
        .await
        .map_err(|_| "M-Lab discovery failed".to_string())?;
    if !response.status().is_success() {
        return Err(format!(
            "M-Lab discovery HTTP {}",
            response.status().as_u16()
        ));
    }
    let body: serde_json::Value = response
        .json()
        .await
        .map_err(|_| "Invalid discovery response")?;
    let entry = &body["results"][0];
    let protocol = if provider == "msak" {
        "throughput/v1"
    } else {
        "ndt/v7"
    };
    let build = |direction: &str| -> Result<String, String> {
        let raw = entry["urls"][format!("wss:///{protocol}/{direction}")]
            .as_str()
            .ok_or("No secure M-Lab endpoint")?;
        let mut url = url::Url::parse(raw).map_err(|_| "Invalid M-Lab endpoint")?;
        if url.scheme() != "wss" {
            return Err("Secure M-Lab transport required".into());
        }
        if provider == "msak" {
            url.query_pairs_mut()
                .append_pair("streams", "2")
                .append_pair("duration", &duration_ms.min(25000).to_string());
        }
        url.query_pairs_mut()
            .append_pair("client_name", "speedqx")
            .append_pair("client_version", "5.0");
        Ok(url.into())
    };
    Ok((
        entry["machine"].as_str().unwrap_or("M-Lab").into(),
        build("download")?,
        build("upload")?,
    ))
}

/// The same existing endpoints and bounded two-at-a-time selection as the web engine.
pub async fn locate_supplementary(
    provider: &str,
    client: &reqwest::Client,
) -> Result<(String, String, String), String> {
    if provider == "cachefly" {
        return Ok((
            "CacheFly".into(),
            "https://cachefly.cachefly.net/100mb.test".into(),
            String::new(),
        ));
    }
    if provider == "fastcom" {
        let response = client
            .get("https://speedqx.com/api/fastcom-targets")
            .timeout(Duration::from_secs(8))
            .send()
            .await
            .map_err(|_| "Existing fast.com relay unavailable")?;
        if !response.status().is_success() {
            return Err(format!("fast.com relay HTTP {}", response.status()));
        }
        let body: serde_json::Value = response
            .json()
            .await
            .map_err(|_| "Invalid relay response")?;
        let target = body["targets"][0].as_str().ok_or("No Netflix target")?;
        let url = url::Url::parse(target).map_err(|_| "Invalid Netflix target")?;
        if url.scheme() != "https"
            || !url.host_str().unwrap_or("").ends_with(".nflxvideo.net")
            || !url.path().contains("/range/")
        {
            return Err("No validated Netflix target".into());
        }
        return Ok((
            url.host_str().unwrap_or("Netflix").into(),
            target.into(),
            String::new(),
        ));
    }
    let candidates: Vec<String> = if provider == "librespeed" {
        ["nyc", "fra", "lon", "atl", "la"]
            .iter()
            .map(|pop| format!("https://{pop}.speedtest.clouvider.net/backend/empty.php?cors=true"))
            .collect()
    } else {
        [
            "nj-us",
            "lax-ca-us",
            "fra-de",
            "sgp",
            "syd-au",
            "tyo-jp",
            "ams-nl",
            "lon-gb",
        ]
        .iter()
        .map(|pop| format!("https://{pop}-ping.vultr.com/vultr.com.100MB.bin"))
        .collect()
    };
    let mut best: Option<(String, Duration)> = None;
    for pair in candidates.chunks(2) {
        for outcome in join_all(pair.iter().map(|url| async move {
            let start = Instant::now();
            let response = client
                .head(url)
                .timeout(Duration::from_millis(1500))
                .send()
                .await
                .ok()?;
            response
                .status()
                .is_success()
                .then(|| (url.clone(), start.elapsed()))
        }))
        .await
        .into_iter()
        .flatten()
        {
            if best.as_ref().is_none_or(|b| outcome.1 < b.1) {
                best = Some(outcome);
            }
        }
    }
    let (target, _) = best.ok_or("No readable public endpoint")?;
    let machine = url::Url::parse(&target)
        .map_err(|_| "Invalid endpoint")?
        .host_str()
        .unwrap_or(provider)
        .to_owned();
    Ok(if provider == "librespeed" {
        (machine, target.replace("empty.php", "garbage.php"), target)
    } else {
        (machine, target, String::new())
    })
}

async fn http_lane(
    provider: &str,
    client: &reqwest::Client,
    endpoint: &str,
    upload: bool,
    budget: &RunBudget,
    counter: &AtomicU64,
    failed: &AtomicBool,
) {
    let mut size = if upload { 1024 } else { 1_000_000 };
    let payload = upload.then(|| synthetic_payload(8_000_000));
    let libre = provider == "librespeed" && !upload;
    let ranged = provider == "cachefly" || provider == "vultr";
    while !budget.stopped() {
        let allocation = budget.reserve(if libre {
            (size / 1_048_576).max(1) * 1_048_576
        } else {
            size
        });
        if libre && !allocation.is_multiple_of(1_048_576) {
            budget.release(allocation);
            break;
        }
        if allocation == 0 {
            break;
        }
        let mut reservation = Reservation {
            budget,
            remaining: allocation,
        };
        let started = Instant::now();
        let mut consumed = 0;
        let mut url = url::Url::parse(endpoint).expect("Static HTTP endpoint");
        url.query_pairs_mut()
            .append_pair("sqx", &budget.started.elapsed().as_nanos().to_string());
        if !upload {
            if libre {
                url.query_pairs_mut()
                    .append_pair("ckSize", &(allocation / 1_048_576).to_string())
                    .append_pair("cors", "true");
            } else if provider == "fastcom" {
                let path = url.path().to_owned();
                if let Some((prefix, rest)) = path.split_once("/range/") {
                    let suffix = rest
                        .split_once('/')
                        .map(|(_, s)| format!("/{s}"))
                        .unwrap_or_default();
                    url.set_path(&format!("{prefix}/range/0-{}{suffix}", allocation - 1));
                }
            } else if !ranged {
                url.query_pairs_mut()
                    .append_pair("bytes", &allocation.to_string());
            }
        }
        let request = if upload {
            budget.consume(allocation, allocation);
            reservation.remaining = 0;
            client
                .post(url)
                .body(payload.as_ref().unwrap()[..allocation as usize].to_vec())
        } else {
            client.get(url)
        };
        let request = if ranged {
            request.header("Range", format!("bytes=0-{}", allocation - 1))
        } else {
            request
        };
        let outcome = async {
            let mut response = request.header("Cache-Control", "no-store").send().await?;
            response.error_for_status_ref()?;
            if ranged && response.status() != reqwest::StatusCode::PARTIAL_CONTENT {
                failed.store(true, Ordering::Relaxed);
                return Ok(());
            }
            if upload {
                counter.fetch_add(allocation, Ordering::Relaxed);
            } else {
                while let Some(chunk) = response.chunk().await? {
                    let n = chunk.len() as u64;
                    consumed += n;
                    counter.fetch_add(n, Ordering::Relaxed);
                    budget.consume(n, n);
                    reservation.remaining = reservation.remaining.saturating_sub(n);
                    if consumed > allocation {
                        failed.store(true, Ordering::Relaxed);
                        break;
                    }
                    if budget.stopped() {
                        break;
                    }
                }
            }
            Ok::<(), reqwest::Error>(())
        }
        .await;
        drop(reservation);
        if outcome.is_err() || failed.load(Ordering::Relaxed) {
            failed.store(true, Ordering::Relaxed);
            break;
        }
        size = (allocation as f64 * if upload { 0.25 } else { 2.0 }
            / started.elapsed().as_secs_f64().max(0.001))
        .min(allocation as f64 * 2.0)
        .round()
        .clamp(if upload { 1024.0 } else { 64_000.0 }, 8_000_000.0) as u64;
    }
}

async fn websocket_lane(
    provider: &str,
    endpoint: &str,
    upload: bool,
    budget: &RunBudget,
    counter: &AtomicU64,
    failed: &AtomicBool,
    metadata: &(AtomicBool, Mutex<Option<f64>>),
) {
    let Ok(mut request) = endpoint.into_client_request() else {
        failed.store(true, Ordering::Relaxed);
        return;
    };
    request.headers_mut().insert(
        "Sec-WebSocket-Protocol",
        if provider == "msak" {
            "net.measurementlab.throughput.v1"
        } else {
            "net.measurementlab.ndt.v7"
        }
        .parse()
        .unwrap(),
    );
    let Ok(Ok((socket, _))) = tokio::time::timeout(
        Duration::from_secs(8),
        tokio_tungstenite::connect_async_with_config(
            request,
            Some(tokio_tungstenite::tungstenite::protocol::WebSocketConfig {
                max_message_size: Some(1 << 24),
                max_frame_size: Some(1 << 24),
                ..Default::default()
            }),
            false,
        ),
    )
    .await
    else {
        failed.store(true, Ordering::Relaxed);
        return;
    };
    let (mut sender, mut receiver) = socket.split();
    let read = async {
        let mut last = 0;
        while let Some(message) = receiver.next().await {
            match message {
                Ok(Message::Binary(bytes)) if !upload => {
                    let n = bytes.len() as u64;
                    counter.fetch_add(n, Ordering::Relaxed);
                    budget.consume(n, 0);
                }
                Ok(Message::Text(text)) => {
                    if let Ok(value) = serde_json::from_str::<serde_json::Value>(&text) {
                        if let Some(rtt) = value["TCPInfo"]["MinRTT"]
                            .as_f64()
                            .filter(|r| r.is_finite() && *r > 0.0)
                        {
                            let mut minimum = metadata.1.lock().unwrap();
                            *minimum = Some(minimum.unwrap_or(f64::INFINITY).min(rtt / 1000.0));
                        }
                        if !upload {
                            continue;
                        }
                        let raw = if provider == "msak" {
                            value.pointer("/Application/BytesReceived")
                        } else {
                            value.pointer("/AppInfo/NumBytes")
                        };
                        if let Some(raw) = raw {
                            let Some(bytes) = raw.as_u64() else {
                                metadata.0.store(true, Ordering::Relaxed);
                                failed.store(true, Ordering::Relaxed);
                                break;
                            };
                            if bytes < last || bytes > 9_007_199_254_740_991 {
                                metadata.0.store(true, Ordering::Relaxed);
                                failed.store(true, Ordering::Relaxed);
                                break;
                            }
                            counter.fetch_add(bytes - last, Ordering::Relaxed);
                            last = bytes;
                        }
                    }
                }
                Ok(Message::Close(_)) => break,
                Err(_) => {
                    failed.store(true, Ordering::Relaxed);
                    break;
                }
                _ => {}
            }
            if budget.stopped() {
                break;
            }
        }
    };
    let write = async {
        if !upload {
            std::future::pending::<()>().await;
        }
        let payload = synthetic_payload(1_048_576);
        let mut frame = 8192;
        let mut sent = 0;
        while !budget.stopped() {
            let n = budget.reserve(frame);
            if n == 0 {
                break;
            }
            let mut reservation = Reservation {
                budget,
                remaining: n,
            };
            if sender
                .send(Message::Binary(payload[..n as usize].to_vec()))
                .await
                .is_err()
            {
                failed.store(true, Ordering::Relaxed);
                break;
            }
            budget.consume(n, n);
            sent += n;
            reservation.remaining = 0;
            if frame < 1_048_576 && sent >= 16 * frame {
                frame *= 2;
            }
        }
    };
    tokio::select! { _ = read => {}, _ = write => {} }
}

pub async fn transfer<F: Fn(f64)>(
    provider: &str,
    endpoint: &str,
    direction: &str,
    duration: Duration,
    client: &reqwest::Client,
    budget: &RunBudget,
    progress: F,
) -> MeasurementTrace {
    let streams = if provider == "ndt7" { 1 } else { 2 };
    let upload = direction == "upload";
    let websocket = matches!(provider, "msak" | "ndt7");
    let mut label = url::Url::parse(endpoint).expect("Validated endpoint");
    label.set_query(None);
    label.set_fragment(None);
    let mut trace = MeasurementTrace {
        provider: provider.into(),
        endpoint: label.into(),
        transport: if websocket { "websocket" } else { "https" }.into(),
        streams,
        direction: direction.into(),
        accounting: if !upload {
            "received"
        } else if websocket {
            "server-received"
        } else {
            "completed-request"
        }
        .into(),
        warmup_ms: WARMUP_MS,
        points: vec![CounterPoint {
            t: 0.0,
            bytes: 0,
            valid: true,
        }],
        stop_reason: "complete".into(),
        integrity_error: None,
        server_tcp_min_rtt_ms: None,
    };
    let counter = AtomicU64::new(0);
    let failed = AtomicBool::new(false);
    let metadata = (AtomicBool::new(false), Mutex::new(None));
    let start = Instant::now();
    let lanes = join_all((0..streams).map(|_| async {
        if websocket {
            websocket_lane(
                provider, endpoint, upload, budget, &counter, &failed, &metadata,
            )
            .await;
        } else {
            http_lane(
                provider, client, endpoint, upload, budget, &counter, &failed,
            )
            .await;
        }
    }));
    tokio::pin!(lanes);
    let mut tick = tokio::time::interval(Duration::from_millis(500));
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    tick.tick().await;
    loop {
        tokio::select! {
            _ = &mut lanes => break,
            _ = budget.wait_stopped() => break,
            _ = tokio::time::sleep_until(tokio::time::Instant::from_std(start + duration)) => break,
            _ = tick.tick() => {
                let elapsed = start.elapsed();
                trace.points.push(CounterPoint { t: elapsed.as_secs_f64() * 1000.0, bytes: counter.load(Ordering::Relaxed), valid: true });
                progress((elapsed.as_secs_f64()/duration.as_secs_f64()).min(1.0));
                if budget.stopped() || elapsed >= duration { break; }
            }
        }
    }
    trace.points.push(CounterPoint {
        t: start.elapsed().as_secs_f64() * 1000.0,
        bytes: counter.load(Ordering::Relaxed),
        valid: true,
    });
    if metadata.0.load(Ordering::Relaxed) {
        trace.integrity_error = Some("Invalid or reset measurement counters".into());
    }
    trace.server_tcp_min_rtt_ms = *metadata.1.lock().unwrap();
    if budget.reason() == "network-change" {
        if let Some(last) = trace.points.last_mut() {
            last.valid = false;
        }
    }
    trace.stop_reason = if budget.stopped() {
        budget.reason()
    } else if failed.load(Ordering::Relaxed) || (websocket && counter.load(Ordering::Relaxed) == 0)
    {
        "failed".into()
    } else {
        "complete".into()
    };
    trace
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reservations_and_stop_reason_are_owned_by_the_run() {
        let budget = RunBudget::new(
            1000,
            Duration::from_secs(90),
            Arc::new(AtomicBool::new(false)),
        );
        assert_eq!(budget.reserve(700), 700);
        assert_eq!(budget.reserve(700), 300);
        assert_eq!(budget.reserve(1), 0);
        assert!(!budget.stopped());
        budget.consume(500, 500);
        budget.release(200);
        assert_eq!(budget.reserve(1000), 200);
        budget.consume(500, 500);
        budget.stop("cancelled");
        assert_eq!(budget.reason(), "byte-limit");
        assert_eq!(budget.used(), 1000);
    }

    #[tokio::test]
    async fn receiver_counter_reset_invalidates_the_socket_trace() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!("ws://{}/ndt/v7/upload", listener.local_addr().unwrap());
        let server = tokio::spawn(async move {
            let (tcp, _) = listener.accept().await.unwrap();
            // The callback's error type is fixed by Tungstenite's handshake API.
            #[expect(clippy::result_large_err, reason = "External callback requires an HTTP error response")]
            let handshake = |_request: &tokio_tungstenite::tungstenite::handshake::server::Request, mut response: tokio_tungstenite::tungstenite::handshake::server::Response| {
                response.headers_mut().insert("Sec-WebSocket-Protocol", "net.measurementlab.ndt.v7".parse().unwrap());
                Ok(response)
            };
            let mut socket = tokio_tungstenite::accept_hdr_async(tcp, handshake)
                .await
                .unwrap();
            socket
                .send(Message::Text(
                    r#"{"AppInfo":{"NumBytes":10000},"TCPInfo":{"MinRTT":12000}}"#.into(),
                ))
                .await
                .unwrap();
            socket
                .send(Message::Text(r#"{"AppInfo":{"NumBytes":5}}"#.into()))
                .await
                .unwrap();
            tokio::time::sleep(Duration::from_millis(100)).await;
        });
        let budget = RunBudget::new(
            500_000_000,
            Duration::from_secs(90),
            Arc::new(AtomicBool::new(false)),
        );
        let trace = transfer(
            "ndt7",
            &endpoint,
            "upload",
            Duration::from_secs(2),
            &client().unwrap(),
            &budget,
            |_| {},
        )
        .await;
        server.await.unwrap();
        assert_eq!(trace.server_tcp_min_rtt_ms, Some(12.0));
        assert_eq!(
            trace.integrity_error.as_deref(),
            Some("Invalid or reset measurement counters")
        );
        assert_eq!(
            super::super::measurement_v5::estimate_trace(&trace).qualification,
            "unavailable"
        );
    }

    #[tokio::test]
    async fn cancelled_connecting_transport_returns_a_partial_record_promptly() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!("ws://{}/ndt", listener.local_addr().unwrap());
        let cancel = Arc::new(AtomicBool::new(false));
        let budget = RunBudget::new(500_000_000, Duration::from_secs(90), cancel.clone());
        let ending = async {
            tokio::time::sleep(Duration::from_millis(30)).await;
            cancel.store(true, Ordering::Relaxed);
        };
        let client = client().unwrap();
        let (trace, ()) = tokio::join!(
            transfer(
                "ndt7",
                &endpoint,
                "download",
                Duration::from_secs(10),
                &client,
                &budget,
                |_| {}
            ),
            ending
        );
        assert_eq!(trace.stop_reason, "cancelled");
        assert!(trace.points.last().unwrap().t < 2000.0);
        assert_eq!(trace.points.last().unwrap().bytes, 0);
    }
}
