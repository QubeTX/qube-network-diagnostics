//! Methodology v5 common profile, independent of the retained v4 reference path.
use super::{
    acquisition_v5::{self, RunBudget},
    measurement_v5::{self, DirectionEstimate, MeasurementTrace},
    *,
};
use std::collections::BTreeMap;

const REFERENCE: &str = "https://speed.cloudflare.com/__down?bytes=0";

// A UDP connect selects a route without sending a packet. Detect source-address
// transitions without running OS commands or changing the user's network.
async fn network_guard(budget: &RunBudget) {
    async fn source() -> Option<std::net::IpAddr> {
        let socket = tokio::net::UdpSocket::bind("0.0.0.0:0").await.ok()?;
        socket.connect("1.1.1.1:443").await.ok()?;
        Some(socket.local_addr().ok()?.ip())
    }
    let initial = source().await;
    loop {
        tokio::time::sleep(Duration::from_secs(1)).await;
        if initial.is_some() && source().await != initial {
            budget.stop("network-change");
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MeasurementSummary {
    pub download: DirectionEstimate,
    pub upload: DirectionEstimate,
    pub primary_providers: BTreeMap<String, Vec<String>>,
    pub traces: Vec<MeasurementTrace>,
    pub bytes_transferred: u64,
    pub budget_bytes: u64,
    pub byte_limit: u64,
    pub elapsed_ms: f64,
    pub stop_reason: String,
}
#[derive(Debug, Clone, Default, Serialize)]
pub struct HttpLatency {
    pub endpoint: String,
    pub idle: Vec<f64>,
    pub download: Vec<f64>,
    pub upload: Vec<f64>,
    pub failures: BTreeMap<String, u64>,
    pub attempts: BTreeMap<String, u64>,
}
async fn probe(client: &reqwest::Client, budget: &RunBudget) -> Option<f64> {
    let start = Instant::now();
    let response = client
        .get(format!(
            "{REFERENCE}&sqx={}",
            budget.started.elapsed().as_nanos()
        ))
        .header("Cache-Control", "no-store")
        .timeout(Duration::from_millis(1500))
        .send()
        .await
        .ok()?;
    if !response.status().is_success() {
        return None;
    }
    response.bytes().await.ok()?;
    Some(start.elapsed().as_secs_f64() * 1000.0)
}
async fn measured_probe(
    client: &reqwest::Client,
    budget: &RunBudget,
    kind: &str,
    latency: &mut HttpLatency,
) {
    if let Some(value) = probe(client, budget).await {
        if budget.stopped() {
            return;
        }
        *latency.attempts.entry(kind.into()).or_default() += 1;
        match kind {
            "idle" => &mut latency.idle,
            "download" => &mut latency.download,
            _ => &mut latency.upload,
        }
        .push(value);
    } else if !budget.stopped() {
        *latency.attempts.entry(kind.into()).or_default() += 1;
        *latency.failures.entry(kind.into()).or_default() += 1;
    }
}
fn phases(provider: &str) -> (Phase, Phase, Phase) {
    match provider {
        "msak" => (Phase::MsakDiscovery, Phase::MsakDownload, Phase::MsakUpload),
        "ndt7" => (Phase::Ndt7Discovery, Phase::Ndt7Download, Phase::Ndt7Upload),
        "librespeed" => (Phase::LsDiscovery, Phase::LsDownload, Phase::LsUpload),
        "cachefly" => (Phase::CfyLatency, Phase::CfyDownload, Phase::CfyDownload),
        "vultr" => (
            Phase::VultrDiscovery,
            Phase::VultrDownload,
            Phase::VultrDownload,
        ),
        "fastcom" => (Phase::FcDiscovery, Phase::FcDownload, Phase::FcUpload),
        _ => (Phase::CfLatency, Phase::CfDownload, Phase::CfUpload),
    }
}
fn provider_result(
    provider: &str,
    server: String,
    traces: &[MeasurementTrace],
    error: Option<String>,
) -> ProviderResult {
    let dl = traces.iter().find(|t| t.direction == "download");
    let ul = traces.iter().find(|t| t.direction == "upload");
    let d = dl.map(measurement_v5::estimate_trace).unwrap_or_default();
    let u = ul.map(measurement_v5::estimate_trace).unwrap_or_default();
    ProviderResult {
        provider: match provider {
            "msak" => "M-Lab MSAK",
            "ndt7" => "M-Lab NDT7",
            "librespeed" => "LibreSpeed",
            "cachefly" => "CacheFly",
            "vultr" => "Vultr",
            "fastcom" => "fast.com",
            _ => "Cloudflare",
        }
        .into(),
        server,
        location: None,
        ping_ms: None,
        jitter_ms: None,
        download_mbps: d.sustained_mbps,
        upload_mbps: u.sustained_mbps,
        download_bytes: dl.and_then(|t| t.points.last()).map_or(0, |p| p.bytes),
        upload_bytes: ul.and_then(|t| t.points.last()).map_or(0, |p| p.bytes),
        download_duration_s: d.measured_ms / 1000.0,
        upload_duration_s: u.measured_ms / 1000.0,
        packet_loss_pct: None,
        error,
        bandwidth_samples: None,
        availability: if d.sustained_mbps.is_none() && u.sustained_mbps.is_none() {
            ProviderAvailability::Failed
        } else {
            ProviderAvailability::Ran
        },
        latency_stats: None,
        loaded_latency: None,
    }
}

pub async fn run<F>(
    config: SpeedTestConfig,
    progress: F,
    on_provider_complete: Option<ProviderCompleteCallback>,
) -> SpeedTestResult
where
    F: Fn(Phase, f64) + Send + Sync + 'static,
{
    let deep = config.provider_set == ProviderSet::All;
    let policy = measurement_v5::profile(deep);
    let budget = RunBudget::new(
        config.max_bytes.unwrap_or(policy.default_max_bytes),
        Duration::from_millis(policy.cap_ms),
        config.cancel.clone(),
    );
    let mut traces = Vec::new();
    let mut providers = Vec::new();
    let mut warnings = Vec::new();
    let mut latency = HttpLatency {
        endpoint: REFERENCE.into(),
        failures: ["idle", "download", "upload"]
            .into_iter()
            .map(|k| (k.into(), 0))
            .collect(),
        attempts: ["idle", "download", "upload"]
            .into_iter()
            .map(|k| (k.into(), 0))
            .collect(),
        ..Default::default()
    };
    let collect = async {
        if let Ok(client) = acquisition_v5::client() {
            progress(Phase::CfLatency, 0.0);
            tokio::select! { _ = probe(&client, &budget) => {}, _ = budget.wait_stopped() => {} }
            for i in 0..12 {
                if budget.stopped() {
                    break;
                }
                tokio::select! { _ = measured_probe(&client, &budget, "idle", &mut latency) => {}, _ = budget.wait_stopped() => {} }
                progress(Phase::CfLatency, (i + 1) as f64 / 12.0);
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
            let mut plan = Vec::new();
            for _ in 0..if deep { 2 } else { 1 } {
                plan.push(("cloudflare", policy.direction_ms / 1000));
                if config.mlab_consent && config.msak_enabled {
                    plan.push(("msak", policy.direction_ms / 1000));
                }
            }
            if config.mlab_consent {
                plan.push(("ndt7", 10));
            } else {
                warnings.push(
                    "M-Lab omitted: data publication consent not granted (--accept-mlab)".into(),
                );
            }
            if deep {
                plan.extend([
                    ("librespeed", 10),
                    ("cachefly", 10),
                    ("vultr", 10),
                    ("fastcom", 10),
                ]);
            }
            let mut mlab_refused = false;
            let mut mlab_endpoints: BTreeMap<&str, (String, String, String)> = BTreeMap::new();
            for (key, seconds) in plan {
                let mlab = matches!(key, "msak" | "ndt7");
                if mlab && mlab_refused {
                    warnings.push(format!("{key} skipped after M-Lab rate limit"));
                    continue;
                }
                if budget.stopped() {
                    break;
                }
                let (discover, dl, ul) = phases(key);
                progress(discover, 0.0);
                let endpoints = if key == "cloudflare" {
                    Ok((
                        "speed.cloudflare.com".into(),
                        "https://speed.cloudflare.com/__down".into(),
                        "https://speed.cloudflare.com/__up".into(),
                    ))
                } else if mlab {
                    if let Some(cached) = mlab_endpoints.get(key) {
                        Ok(cached.clone())
                    } else {
                        let located = tokio::select! { endpoints = acquisition_v5::locate(key, seconds*1000, &client) => endpoints, _ = budget.wait_stopped() => Err("Run ended during discovery".into()) };
                        if let Ok(endpoints) = &located {
                            mlab_endpoints.insert(key, endpoints.clone());
                        }
                        located
                    }
                } else {
                    tokio::select! { endpoints = acquisition_v5::locate_supplementary(key, &client) => endpoints, _ = budget.wait_stopped() => Err("Run ended during discovery".into()) }
                };
                let first = traces.len();
                let (server, error) = match endpoints {
                    Ok((server, download, upload)) => {
                        for (direction, endpoint, phase) in
                            [("download", download, dl), ("upload", upload, ul)]
                        {
                            if budget.stopped() {
                                break;
                            }
                            if endpoint.is_empty() {
                                continue;
                            }
                            progress(phase, 0.0);
                            let transfer = acquisition_v5::transfer(
                                key,
                                &endpoint,
                                direction,
                                Duration::from_secs(seconds),
                                &client,
                                &budget,
                                |p| progress(phase, p),
                            );
                            let probing = async {
                                loop {
                                    measured_probe(&client, &budget, direction, &mut latency).await;
                                    tokio::time::sleep(Duration::from_millis(500)).await;
                                }
                            };
                            tokio::pin!(transfer);
                            tokio::pin!(probing);
                            let trace = tokio::select! { trace = &mut transfer => trace, _ = &mut probing => unreachable!() };
                            traces.push(trace);
                            progress(phase, 1.0);
                        }
                        (server, None)
                    }
                    Err(error) => {
                        if mlab && error.contains("429") {
                            mlab_refused = true;
                        }
                        warnings.push(format!("{key}: {error}"));
                        (String::new(), Some(error))
                    }
                };
                let provider = provider_result(key, server, &traces[first..], error);
                if let Some(callback) = &on_provider_complete {
                    callback(&provider);
                }
                providers.push(provider);
                if !budget.stopped() {
                    tokio::select! { _ = tokio::time::sleep(Duration::from_millis(500)) => {}, _ = budget.wait_stopped() => {} }
                }
            }
        } else {
            warnings.push("Unable to initialize HTTP transport".into());
        }
    };
    tokio::select! { _ = collect => {}, _ = network_guard(&budget) => unreachable!() }
    let (download, dp) = measurement_v5::summarize_traces(&traces, "download");
    let (upload, up) = measurement_v5::summarize_traces(&traces, "upload");
    warnings.extend(download.warnings.iter().chain(&upload.warnings).cloned());
    let reason = budget.reason();
    if reason != "complete" {
        warnings.push(format!("Run ended: {reason}"));
    }
    let stats = LatencyStats::from_rtts(&latency.idle).map(|mut stats| {
        // v5 reports the empirical population spread, matching the shared TS model.
        stats.stddev = (latency
            .idle
            .iter()
            .map(|r| (r - stats.mean).powi(2))
            .sum::<f64>()
            / latency.idle.len() as f64)
            .sqrt();
        stats
    });
    let result = SpeedTestResult {
        methodology_version: "5.0",
        platform: "cli",
        provider_set: if deep { "full" } else { "fast" },
        ping_ms: stats.as_ref().map(|s| s.p50),
        jitter_ms: stats.as_ref().map(|s| s.pdv),
        jitter_rfc3550: stats.as_ref().map(|s| s.jitter_rfc3550),
        download_mbps: download.sustained_mbps.unwrap_or(0.0),
        upload_mbps: upload.sustained_mbps.unwrap_or(0.0),
        packet_loss_pct: None,
        capacity: empty_direction(),
        consensus: empty_direction(),
        agreement: AgreementInfo {
            i2: None,
            band: statistics::AgreementBand::Insufficient,
        },
        upload_agreement: None,
        rpm: None,
        bufferbloat: None,
        latency_stats: stats,
        providers,
        duration_s: budget.started.elapsed().as_secs_f64(),
        stability: None,
        provider_divergence: None,
        confidence_intervals: None,
        merge_exclusions: vec![],
        measurement: Some(MeasurementSummary {
            download,
            upload,
            primary_providers: [("download".into(), dp), ("upload".into(), up)].into(),
            bytes_transferred: traces
                .iter()
                .filter_map(|t| t.points.last())
                .map(|p| p.bytes)
                .sum(),
            traces,
            budget_bytes: budget.used(),
            byte_limit: budget.limit,
            elapsed_ms: budget.started.elapsed().as_secs_f64() * 1000.0,
            stop_reason: reason,
        }),
        http_latency: Some(latency),
        warnings,
    };
    progress(Phase::Computing, 1.0);
    result
}
