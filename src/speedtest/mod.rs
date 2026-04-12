pub mod cloudflare;
pub mod display;
pub mod fastcom;
pub mod librespeed;
pub mod ndt7;
pub mod statistics;

use serde::Serialize;
use std::sync::Arc;
use std::time::Instant;

/// Test duration configuration
#[derive(Debug, Clone)]
pub enum TestDuration {
    /// Fixed duration per direction in seconds (e.g., 30 = 30s download + 30s upload)
    Seconds(u64),
    /// Let providers use their natural duration
    Auto,
}

/// Which providers to run
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ProviderSet {
    /// All 4 providers: Cloudflare, NDT7, LibreSpeed, fast.com (speedqx default)
    All,
    /// Diagnostic subset: Cloudflare + NDT7 only (nd300 default)
    Diagnostic,
}

/// Configuration for the speed test orchestrator
#[derive(Debug, Clone)]
pub struct SpeedTestConfig {
    /// Duration per direction for CF, NDT7, LibreSpeed (default: 30s)
    pub duration: TestDuration,
    /// Duration per direction for fast.com (default: Auto)
    pub fastcom_duration: TestDuration,
    /// Number of latency probes
    pub latency_probes: u32,
    /// Which providers to run
    pub provider_set: ProviderSet,
    /// Enable colored output
    pub use_colors: bool,
}

impl Default for SpeedTestConfig {
    fn default() -> Self {
        Self {
            duration: TestDuration::Seconds(30),
            fastcom_duration: TestDuration::Auto,
            latency_probes: 20,
            provider_set: ProviderSet::All,
            use_colors: true,
        }
    }
}

/// Phase indicator for progress callbacks
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Phase {
    CfLatency,
    CfDownload,
    CfUpload,
    Ndt7Discovery,
    Ndt7Download,
    Ndt7Upload,
    LsDiscovery,
    LsDownload,
    LsUpload,
    FcDiscovery,
    FcDownload,
    FcUpload,
    Computing,
}

/// Raw per-request Mbps samples for statistical post-processing.
#[derive(Debug, Clone, Default, Serialize)]
pub struct BandwidthSamples {
    pub download: Vec<f64>,
    pub upload: Vec<f64>,
}

/// Connection stability metrics (coefficient of variation).
#[derive(Debug, Clone, Serialize)]
pub struct StabilityMetrics {
    pub download_cv: f64,
    pub upload_cv: f64,
    pub download_stable: bool,
    pub upload_stable: bool,
}

/// Provider divergence detection.
#[derive(Debug, Clone, Serialize)]
pub struct ProviderDivergence {
    pub download: f64,
    pub upload: f64,
    pub significant: bool,
}

/// Per-provider speed test result
#[derive(Debug, Clone, Serialize)]
pub struct ProviderResult {
    pub provider: String,
    pub server: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub location: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ping_ms: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub jitter_ms: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub download_mbps: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub upload_mbps: Option<f64>,
    pub download_bytes: u64,
    pub upload_bytes: u64,
    pub download_duration_s: f64,
    pub upload_duration_s: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub packet_loss_pct: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bandwidth_samples: Option<BandwidthSamples>,
}

/// Aggregated speed test result (used by both speedqx and nd300)
#[derive(Debug, Clone, Serialize)]
pub struct SpeedTestResult {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ping_ms: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub jitter_ms: Option<f64>,
    pub download_mbps: f64,
    pub upload_mbps: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub packet_loss_pct: Option<f64>,
    pub providers: Vec<ProviderResult>,
    pub duration_s: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stability: Option<StabilityMetrics>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider_divergence: Option<ProviderDivergence>,
}

/// Latency weight for Cloudflare (NDT7 gets 1 - this).
/// NDT7's MinRTT from TCP kernel is structurally superior, not just lower-variance.
const CF_LATENCY_WEIGHT: f64 = 0.4;

/// Divergence threshold: flag when providers differ by more than this fraction.
const DIVERGENCE_THRESHOLD: f64 = 0.3;

fn divergence_ratio(a: f64, b: f64) -> f64 {
    if a <= 0.0 || b <= 0.0 {
        return 0.0;
    }
    (a - b).abs() / a.max(b)
}

/// Aggregation result including new metrics.
struct AggregateResult {
    ping: Option<f64>,
    jitter: Option<f64>,
    download: f64,
    upload: f64,
    packet_loss: Option<f64>,
    stability: Option<StabilityMetrics>,
    divergence: Option<ProviderDivergence>,
}

/// Inverse-variance weighted aggregation across providers.
/// Uses accurate bandwidth pipeline on raw samples, fixed latency weights,
/// stability metrics, and divergence detection.
fn aggregate(providers: &[ProviderResult]) -> AggregateResult {
    let successful: Vec<&ProviderResult> = providers.iter().filter(|p| p.error.is_none()).collect();

    if successful.is_empty() {
        return AggregateResult {
            ping: None, jitter: None, download: 0.0, upload: 0.0,
            packet_loss: None, stability: None, divergence: None,
        };
    }

    // ── Compute accurate bandwidth per provider from raw samples ────
    let mut provider_dl: Vec<(f64, f64)> = Vec::new(); // (accurate_mbps, variance)
    let mut provider_ul: Vec<(f64, f64)> = Vec::new();
    let mut all_dl_samples: Vec<f64> = Vec::new();
    let mut all_ul_samples: Vec<f64> = Vec::new();

    for p in &successful {
        if let Some(ref samples) = p.bandwidth_samples {
            if !samples.download.is_empty() {
                let acc = statistics::accurate_bandwidth(&samples.download);
                let var = statistics::variance(&samples.download);
                if acc > 0.0 {
                    provider_dl.push((acc, var));
                }
                all_dl_samples.extend_from_slice(&samples.download);
            }
            if !samples.upload.is_empty() {
                let acc = statistics::accurate_upload_bandwidth(&samples.upload);
                let var = statistics::variance(&samples.upload);
                if acc > 0.0 {
                    provider_ul.push((acc, var));
                }
                all_ul_samples.extend_from_slice(&samples.upload);
            }
        }
        // Fallback: use provider-reported value if no raw samples
        if p.bandwidth_samples.is_none() || p.bandwidth_samples.as_ref().map_or(true, |s| s.download.is_empty()) {
            if let Some(dl) = p.download_mbps {
                if dl > 0.0 {
                    provider_dl.push((dl, 0.0));
                }
            }
        }
        if p.bandwidth_samples.is_none() || p.bandwidth_samples.as_ref().map_or(true, |s| s.upload.is_empty()) {
            if let Some(ul) = p.upload_mbps {
                if ul > 0.0 {
                    provider_ul.push((ul, 0.0));
                }
            }
        }
    }

    // ── Merge bandwidth via inverse-variance weighting ─────────────
    let download = if provider_dl.is_empty() {
        0.0
    } else if provider_dl.len() == 1 {
        provider_dl[0].0
    } else {
        // Pairwise merge
        let mut acc = provider_dl[0];
        for pair in &provider_dl[1..] {
            let merged = statistics::inverse_variance_merge(acc.0, acc.1, pair.0, pair.1);
            acc = (merged.value, 0.0); // variance of merged not needed for further pairwise
        }
        acc.0
    };

    let upload = if provider_ul.is_empty() {
        0.0
    } else if provider_ul.len() == 1 {
        provider_ul[0].0
    } else {
        let mut acc = provider_ul[0];
        for pair in &provider_ul[1..] {
            let merged = statistics::inverse_variance_merge(acc.0, acc.1, pair.0, pair.1);
            acc = (merged.value, 0.0);
        }
        acc.0
    };

    // ── Latency: confidence-weighted merge (CF 0.4 / NDT7 0.6) ─────
    let cf_ping = successful.iter().find(|p| p.provider == "Cloudflare").and_then(|p| p.ping_ms);
    let ndt_ping = successful.iter().find(|p| p.provider == "M-Lab NDT7").and_then(|p| p.ping_ms);
    let cf_jitter = successful.iter().find(|p| p.provider == "Cloudflare").and_then(|p| p.jitter_ms);
    let ndt_jitter = successful.iter().find(|p| p.provider == "M-Lab NDT7").and_then(|p| p.jitter_ms);

    let ping = match (cf_ping, ndt_ping) {
        (Some(cf), Some(ndt)) => Some(statistics::weighted_merge(cf, ndt, CF_LATENCY_WEIGHT)),
        (Some(cf), None) => Some(cf),
        (None, Some(ndt)) => Some(ndt),
        (None, None) => {
            // Fallback: minimum across all providers
            successful.iter()
                .filter_map(|p| p.ping_ms)
                .filter(|p| *p > 0.0)
                .min_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal))
        }
    };

    let jitter = match (cf_jitter, ndt_jitter) {
        (Some(cf), Some(ndt)) => Some(statistics::weighted_merge(cf, ndt, CF_LATENCY_WEIGHT)),
        (Some(cf), None) => Some(cf),
        (None, Some(ndt)) => Some(ndt),
        (None, None) => {
            let jitters: Vec<f64> = successful.iter().filter_map(|p| p.jitter_ms).filter(|j| *j > 0.0).collect();
            if jitters.is_empty() { None } else { Some(statistics::mean(&jitters)) }
        }
    };

    // Packet loss from Cloudflare (only provider that measures it)
    let packet_loss = successful
        .iter()
        .find(|p| p.provider == "Cloudflare")
        .and_then(|p| p.packet_loss_pct);

    // ── Stability metrics ──────────────────────────────────────────
    let stability = if all_dl_samples.len() > 2 || all_ul_samples.len() > 2 {
        let dl_cv = statistics::coefficient_of_variation(&all_dl_samples);
        let ul_cv = statistics::coefficient_of_variation(&all_ul_samples);
        Some(StabilityMetrics {
            download_cv: dl_cv,
            upload_cv: ul_cv,
            download_stable: dl_cv < 0.15,
            upload_stable: ul_cv < 0.15,
        })
    } else {
        None
    };

    // ── Provider divergence ────────────────────────────────────────
    let divergence = if provider_dl.len() >= 2 || provider_ul.len() >= 2 {
        let dl_div = if provider_dl.len() >= 2 {
            divergence_ratio(provider_dl[0].0, provider_dl[1].0)
        } else {
            0.0
        };
        let ul_div = if provider_ul.len() >= 2 {
            divergence_ratio(provider_ul[0].0, provider_ul[1].0)
        } else {
            0.0
        };
        Some(ProviderDivergence {
            download: dl_div,
            upload: ul_div,
            significant: dl_div > DIVERGENCE_THRESHOLD || ul_div > DIVERGENCE_THRESHOLD,
        })
    } else {
        None
    };

    AggregateResult {
        ping, jitter, download, upload, packet_loss, stability, divergence,
    }
}

/// Callback type for provider completion notifications.
pub type ProviderCompleteCallback = Arc<dyn Fn(&ProviderResult) + Send + Sync>;

/// Run the speed test with the given configuration and progress callback.
/// The `on_provider_complete` callback is called after each provider finishes,
/// allowing the UI to show per-provider summaries.
pub async fn run<F>(
    config: SpeedTestConfig,
    progress: F,
    on_provider_complete: Option<ProviderCompleteCallback>,
) -> SpeedTestResult
where
    F: Fn(Phase, f64) + Send + Sync + 'static,
{
    let start = Instant::now();
    let mut providers = Vec::new();
    let progress = Arc::new(progress);

    // Cloudflare (always runs)
    {
        let pg = progress.clone();
        let cf_result = cloudflare::run(&config, move |phase, p| pg(phase, p)).await;
        if let Some(ref cb) = on_provider_complete {
            cb(&cf_result);
        }
        providers.push(cf_result);
    }

    // M-Lab NDT7 (always runs)
    {
        let pg = progress.clone();
        let ndt_result = ndt7::run(&config, move |phase, p| pg(phase, p)).await;
        if let Some(ref cb) = on_provider_complete {
            cb(&ndt_result);
        }
        providers.push(ndt_result);
    }

    // LibreSpeed + fast.com (only in All mode)
    if config.provider_set == ProviderSet::All {
        {
            let pg = progress.clone();
            let ls_result = librespeed::run(&config, move |phase, p| pg(phase, p)).await;
            if let Some(ref cb) = on_provider_complete {
                cb(&ls_result);
            }
            providers.push(ls_result);
        }

        {
            let pg = progress.clone();
            let fc_result = fastcom::run(&config, move |phase, p| pg(phase, p)).await;
            if let Some(ref cb) = on_provider_complete {
                cb(&fc_result);
            }
            providers.push(fc_result);
        }
    }

    progress(Phase::Computing, 1.0);

    let agg = aggregate(&providers);
    let duration = start.elapsed().as_secs_f64();

    SpeedTestResult {
        ping_ms: agg.ping,
        jitter_ms: agg.jitter,
        download_mbps: agg.download,
        upload_mbps: agg.upload,
        packet_loss_pct: agg.packet_loss,
        providers,
        duration_s: duration,
        stability: agg.stability,
        provider_divergence: agg.divergence,
    }
}

/// Format Mbps value for display
pub fn format_mbps(mbps: f64) -> String {
    if mbps >= 1000.0 {
        format!("{:.1} Gbps", mbps / 1000.0)
    } else if mbps >= 100.0 {
        format!("{:.0} Mbps", mbps)
    } else if mbps >= 10.0 {
        format!("{:.1} Mbps", mbps)
    } else {
        format!("{:.2} Mbps", mbps)
    }
}

/// Format bytes for display
pub fn format_bytes(bytes: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = 1024 * KB;
    const GB: u64 = 1024 * MB;

    if bytes >= GB {
        format!("{:.2} GB", bytes as f64 / GB as f64)
    } else if bytes >= MB {
        format!("{:.1} MB", bytes as f64 / MB as f64)
    } else if bytes >= KB {
        format!("{:.1} KB", bytes as f64 / KB as f64)
    } else {
        format!("{} B", bytes)
    }
}
