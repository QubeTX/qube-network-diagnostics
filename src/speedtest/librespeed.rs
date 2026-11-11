use super::adaptive::adaptive_chunk_bytes;
use super::{
    statistics, BandwidthSamples, LatencyStats, Phase, ProviderAvailability, ProviderResult,
    SpeedTestConfig, TestDuration,
};
use reqwest::Client;
use serde::Deserialize;
use std::time::{Duration, Instant};

/// Cache-busting variant of a URL (appends a unique nonce query param).
fn cache_bust(url: &str) -> String {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let sep = if url.contains('?') { '&' } else { '?' };
    format!("{url}{sep}nocache={nanos}")
}

const SERVER_LIST_URL: &str = "https://librespeed.org/backend-servers/servers.json";

/// Maximum servers to probe for latency during selection. Probes run
/// concurrently with 5s timeouts, so breadth costs little wall time and a
/// nearby server is far more likely to be in the candidate set.
const MAX_SERVERS_TO_PROBE: usize = 30;

/// Each transfer request is sized to take roughly this long at the link's
/// most recently measured throughput, keeping the per-request sample rate
/// constant across link speeds (the old fixed 100 MB chunks yielded 0-2
/// samples per 30s window on slower lines).
const TARGET_REQUEST_SECS: f64 = 2.0;

/// Download request bounds (ckSize is in MB units).
const DOWNLOAD_MIN_BYTES: u64 = 1_000_000;
const DOWNLOAD_MAX_BYTES: u64 = 100_000_000;

/// Upload request bounds. 16 MB stays under common LibreSpeed POST limits.
const UPLOAD_MIN_BYTES: u64 = 256 * 1024;
const UPLOAD_MAX_BYTES: u64 = 16_000_000;

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

/// Fallback servers if the server list fetch fails. Geo-diverse so a user on
/// any continent gets a plausible candidate (RTT-based selection picks the
/// nearest); entries pinned from the live backend-servers list (2026-06),
/// preferring the official librespeed.org backends and large long-lived
/// operators.
const FALLBACK_SERVERS: &[FallbackServer] = &[
    FallbackServer {
        name: "New York, US (Clouvider)",
        server: "https://nyc.speedtest.clouvider.net/backend",
        dl_url: "garbage.php",
        ul_url: "empty.php",
        ping_url: "empty.php",
    },
    FallbackServer {
        name: "Los Angeles, US (Clouvider)",
        server: "https://la.speedtest.clouvider.net/backend",
        dl_url: "garbage.php",
        ul_url: "empty.php",
        ping_url: "empty.php",
    },
    FallbackServer {
        name: "Atlanta, US (Clouvider)",
        server: "https://atl.speedtest.clouvider.net/backend",
        dl_url: "garbage.php",
        ul_url: "empty.php",
        ping_url: "empty.php",
    },
    FallbackServer {
        name: "Frankfurt, Germany (Clouvider)",
        server: "https://fra.speedtest.clouvider.net/backend",
        dl_url: "garbage.php",
        ul_url: "empty.php",
        ping_url: "empty.php",
    },
    FallbackServer {
        name: "London, England (Clouvider)",
        server: "https://lon.speedtest.clouvider.net/backend",
        dl_url: "garbage.php",
        ul_url: "empty.php",
        ping_url: "empty.php",
    },
    FallbackServer {
        name: "Bangalore, India (librespeed.org)",
        server: "https://in1.backend.librespeed.org",
        dl_url: "garbage.php",
        ul_url: "empty.php",
        ping_url: "empty.php",
    },
    FallbackServer {
        name: "Singapore (DS Group)",
        server: "https://speedtest.dsgroupmedia.com",
        dl_url: "backend/garbage.php",
        ul_url: "backend/empty.php",
        ping_url: "backend/empty.php",
    },
    FallbackServer {
        name: "Tokyo, Japan (A573)",
        server: "https://librespeed.a573.net",
        dl_url: "backend/garbage.php",
        ul_url: "backend/empty.php",
        ping_url: "backend/empty.php",
    },
    FallbackServer {
        name: "Johannesburg, South Africa (librespeed.org)",
        server: "https://za1.backend.librespeed.org",
        dl_url: "garbage.php",
        ul_url: "empty.php",
        ping_url: "empty.php",
    },
];

struct FallbackServer {
    name: &'static str,
    server: &'static str,
    dl_url: &'static str,
    ul_url: &'static str,
    ping_url: &'static str,
}

#[derive(Debug, Deserialize)]
struct ServerEntry {
    #[allow(dead_code)]
    id: Option<u64>,
    name: Option<String>,
    server: String,
    #[serde(rename = "dlURL")]
    dl_url: String,
    #[serde(rename = "ulURL")]
    ul_url: String,
    #[serde(rename = "pingURL")]
    ping_url: String,
    #[serde(rename = "getIpURL")]
    #[allow(dead_code)]
    get_ip_url: Option<String>,
}

struct SelectedServer {
    name: String,
    base_url: String,
    dl_url: String,
    ul_url: String,
    ping_url: String,
    #[allow(dead_code)]
    ping_ms: f64,
}

/// Run the LibreSpeed speed test: server discovery, latency, download, upload.
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
    progress(Phase::LsDiscovery, 0.0);

    let servers = fetch_server_list(&client).await;
    let selected = select_best_server(&client, &servers, progress).await?;

    progress(Phase::LsDiscovery, 1.0);

    // ── Duration split ───────────────────────────────────────────────
    let (dl_secs, ul_secs) = match &config.duration {
        TestDuration::Seconds(s) => (*s, *s),
        TestDuration::Auto => (15, 15),
    };

    // ── Dense idle-latency engine (METHODOLOGY.md §4) ────────────────
    let n_probes = super::dense_probe_count(dl_secs as f64);
    let base_ping_url = format!("{}/{}", selected.base_url, selected.ping_url);
    let mut rtts: Vec<f64> = Vec::with_capacity(n_probes as usize);
    let mut failures: u32 = 0;

    for i in 0..n_probes {
        let start = Instant::now();
        match client
            .head(cache_bust(&base_ping_url))
            .timeout(Duration::from_secs(5))
            .send()
            .await
        {
            Ok(_) => rtts.push(start.elapsed().as_secs_f64() * 1000.0),
            Err(_) => failures += 1,
        }
        if i + 1 < n_probes {
            tokio::time::sleep(super::DENSE_PROBE_INTERVAL).await;
        }
    }

    // Discard the first 3 warm-up probes; keep all if ≤ 3.
    let measured: Vec<f64> = if rtts.len() > super::DENSE_PROBE_WARMUP {
        rtts[super::DENSE_PROBE_WARMUP..].to_vec()
    } else {
        rtts.clone()
    };
    let latency_stats = LatencyStats::from_rtts(&measured);
    let ping_ms = measured
        .iter()
        .copied()
        .min_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let jitter_ms = latency_stats.as_ref().map(|ls| ls.pdv);

    let packet_loss_pct = if n_probes > 0 {
        Some(failures as f64 / n_probes as f64 * 100.0)
    } else {
        None
    };

    // ── Download phase ───────────────────────────────────────────────
    progress(Phase::LsDownload, 0.0);

    let dl_deadline = Instant::now() + Duration::from_secs(dl_secs);
    let mut dl_bytes: u64 = 0;
    let dl_start = Instant::now();
    let mut dl_mbps_samples: Vec<f64> = Vec::new();
    // Adaptive request sizing: seed small (warmup — the slow-start discard
    // absorbs it), then size each request to ~TARGET_REQUEST_SECS at the
    // throughput the previous request measured.
    let mut dl_chunk_bytes: u64 = DOWNLOAD_MIN_BYTES;

    while Instant::now() < dl_deadline {
        // ckSize is in MB units on LibreSpeed backends.
        let ck_size = (dl_chunk_bytes / 1_000_000).max(1);
        let dl_url = format!(
            "{}/{}?ckSize={}",
            selected.base_url, selected.dl_url, ck_size
        );
        let req_start = Instant::now();
        match client
            .get(&dl_url)
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
                        let mbps = (req_bytes as f64 * 8.0) / (req_duration * 1_000_000.0);
                        dl_mbps_samples.push(mbps);
                        dl_chunk_bytes = adaptive_chunk_bytes(
                            mbps,
                            TARGET_REQUEST_SECS,
                            DOWNLOAD_MIN_BYTES,
                            DOWNLOAD_MAX_BYTES,
                        );
                    }
                    let elapsed = dl_start.elapsed().as_secs_f64();
                    let frac = (elapsed / dl_secs as f64).min(1.0);
                    progress(Phase::LsDownload, frac);
                }
            }
            Err(_) => {
                // Re-warm after an error: the link state is unknown.
                dl_chunk_bytes = DOWNLOAD_MIN_BYTES;
            }
            _ => {}
        }
    }

    let dl_elapsed = dl_start.elapsed().as_secs_f64();
    progress(Phase::LsDownload, 1.0);

    let download_mbps = if dl_mbps_samples.is_empty() {
        None
    } else {
        Some(statistics::accurate_bandwidth(&dl_mbps_samples))
    };

    // ── Upload phase ─────────────────────────────────────────────────
    progress(Phase::LsUpload, 0.0);

    let ul_url = format!("{}/{}", selected.base_url, selected.ul_url);
    let ul_deadline = Instant::now() + Duration::from_secs(ul_secs);
    let mut ul_bytes: u64 = 0;
    let ul_start = Instant::now();
    let mut ul_mbps_samples: Vec<f64> = Vec::new();
    // Adaptive payload sizing, same policy as download; the buffer is only
    // rebuilt when the size actually changes.
    let mut ul_chunk_bytes: u64 = UPLOAD_MIN_BYTES.max(1_000_000);
    let mut upload_payload = vec![0u8; ul_chunk_bytes as usize];

    while Instant::now() < ul_deadline {
        if upload_payload.len() as u64 != ul_chunk_bytes {
            upload_payload = vec![0u8; ul_chunk_bytes as usize];
        }
        let req_bytes = upload_payload.len() as u64;
        let req_start = Instant::now();
        match client
            .post(&ul_url)
            .body(upload_payload.clone())
            .timeout(remaining_budget(ul_deadline))
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
                progress(Phase::LsUpload, frac);
            }
            Err(_) => {
                ul_chunk_bytes = UPLOAD_MIN_BYTES;
            }
            _ => {}
        }
    }

    let ul_elapsed = ul_start.elapsed().as_secs_f64();
    progress(Phase::LsUpload, 1.0);

    let upload_mbps = if ul_mbps_samples.is_empty() {
        None
    } else {
        Some(statistics::accurate_upload_bandwidth(&ul_mbps_samples))
    };

    // Honest failure: server selection succeeded but neither direction produced
    // a usable transfer sample. Report an explicit error so the speedqx table
    // shows an error row (and aggregation excludes this provider).
    let error = if dl_mbps_samples.is_empty() && ul_mbps_samples.is_empty() {
        Some("no successful transfers".to_string())
    } else {
        None
    };
    let availability = if error.is_some() {
        ProviderAvailability::Failed
    } else {
        ProviderAvailability::Ran
    };

    Ok(ProviderResult {
        provider: "LibreSpeed".to_string(),
        server: selected.name.clone(),
        location: Some(selected.base_url.clone()),
        ping_ms,
        jitter_ms,
        download_mbps,
        upload_mbps,
        download_bytes: dl_bytes,
        upload_bytes: ul_bytes,
        download_duration_s: dl_elapsed,
        upload_duration_s: ul_elapsed,
        packet_loss_pct,
        error,
        bandwidth_samples: Some(BandwidthSamples {
            download: dl_mbps_samples,
            upload: ul_mbps_samples,
        }),
        availability,
        latency_stats,
        loaded_latency: None,
    })
}

/// Fetch the public LibreSpeed server list. Falls back to hardcoded servers.
async fn fetch_server_list(client: &Client) -> Vec<ServerEntry> {
    match client
        .get(SERVER_LIST_URL)
        .timeout(Duration::from_secs(10))
        .send()
        .await
    {
        Ok(resp) => match resp.json::<Vec<ServerEntry>>().await {
            Ok(servers) if !servers.is_empty() => servers,
            _ => fallback_servers(),
        },
        Err(_) => fallback_servers(),
    }
}

fn fallback_servers() -> Vec<ServerEntry> {
    FALLBACK_SERVERS
        .iter()
        .enumerate()
        .map(|(i, s)| ServerEntry {
            id: Some(i as u64 + 900),
            name: Some(s.name.to_string()),
            server: s.server.to_string(),
            dl_url: s.dl_url.to_string(),
            ul_url: s.ul_url.to_string(),
            ping_url: s.ping_url.to_string(),
            get_ip_url: None,
        })
        .collect()
}

/// Select the best server by probing latency on up to MAX_SERVERS_TO_PROBE servers.
async fn select_best_server<F>(
    client: &Client,
    servers: &[ServerEntry],
    progress: &F,
) -> Result<SelectedServer, String>
where
    F: Fn(Phase, f64) + Send + Sync,
{
    let candidates: Vec<&ServerEntry> = servers.iter().take(MAX_SERVERS_TO_PROBE).collect();

    if candidates.is_empty() {
        return Err("LibreSpeed: no servers available".to_string());
    }

    // Probe all candidates concurrently
    let mut handles = Vec::new();
    for entry in &candidates {
        let ping_url = format!("{}/{}", entry.server, entry.ping_url);
        let client = client.clone();
        handles.push(tokio::spawn(async move {
            let start = Instant::now();
            match client
                .head(&ping_url)
                .timeout(Duration::from_secs(5))
                .send()
                .await
            {
                Ok(_) => Some(start.elapsed().as_secs_f64() * 1000.0),
                Err(_) => None,
            }
        }));
    }

    let mut best_idx = 0;
    let mut best_rtt = f64::MAX;

    for (i, handle) in handles.into_iter().enumerate() {
        if let Ok(Some(rtt)) = handle.await {
            if rtt < best_rtt {
                best_rtt = rtt;
                best_idx = i;
            }
        }
        let frac = (i + 1) as f64 / candidates.len() as f64;
        progress(Phase::LsDiscovery, frac * 0.9); // 0-90% for probing
    }

    if best_rtt == f64::MAX {
        return Err("LibreSpeed: all server probes failed".to_string());
    }

    let entry = candidates[best_idx];
    Ok(SelectedServer {
        name: entry.name.clone().unwrap_or_else(|| entry.server.clone()),
        base_url: entry.server.clone(),
        dl_url: entry.dl_url.clone(),
        ul_url: entry.ul_url.clone(),
        ping_url: entry.ping_url.clone(),
        ping_ms: best_rtt,
    })
}

fn error_result(msg: String) -> ProviderResult {
    ProviderResult {
        provider: "LibreSpeed".to_string(),
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
