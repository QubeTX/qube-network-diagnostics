pub mod cloudflare;
pub mod display;
pub mod fastcom;
pub mod librespeed;
pub mod ndt7;

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
}

/// Remove outliers from a sample set using the IQR method.
pub fn remove_outliers(values: &[f64]) -> Vec<f64> {
    if values.len() < 4 {
        return values.to_vec();
    }
    let mut sorted = values.to_vec();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let q1 = sorted[sorted.len() / 4];
    let q3 = sorted[3 * sorted.len() / 4];
    let iqr = q3 - q1;
    let lower = q1 - 1.5 * iqr;
    let upper = q3 + 1.5 * iqr;
    sorted
        .into_iter()
        .filter(|v| *v >= lower && *v <= upper)
        .collect()
}

/// Volume-weighted aggregation across providers.
/// Uses minimum ping, mean jitter, volume-weighted download/upload.
fn aggregate(providers: &[ProviderResult]) -> (Option<f64>, Option<f64>, f64, f64, Option<f64>) {
    let successful: Vec<&ProviderResult> = providers.iter().filter(|p| p.error.is_none()).collect();

    if successful.is_empty() {
        return (None, None, 0.0, 0.0, None);
    }

    // Ping: minimum across all providers (best represents physical link latency)
    let pings: Vec<f64> = successful
        .iter()
        .filter_map(|p| p.ping_ms)
        .filter(|p| *p > 0.0)
        .collect();

    let ping = if pings.is_empty() {
        None
    } else {
        pings
            .iter()
            .copied()
            .min_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal))
    };

    // Jitter: mean across providers
    let jitters: Vec<f64> = successful
        .iter()
        .filter_map(|p| p.jitter_ms)
        .filter(|j| *j > 0.0)
        .collect();

    let jitter = if jitters.is_empty() {
        None
    } else {
        Some(jitters.iter().sum::<f64>() / jitters.len() as f64)
    };

    // Download: volume-weighted average (weight by bytes transferred)
    let download = {
        let pairs: Vec<(f64, u64)> = successful
            .iter()
            .filter_map(|p| p.download_mbps.map(|dl| (dl, p.download_bytes)))
            .filter(|(dl, _)| *dl > 0.0)
            .collect();
        let total_bytes: u64 = pairs.iter().map(|(_, b)| *b).sum();
        if total_bytes > 0 {
            pairs
                .iter()
                .map(|(dl, b)| dl * *b as f64)
                .sum::<f64>()
                / total_bytes as f64
        } else if !pairs.is_empty() {
            // Fallback to simple mean if no byte data
            pairs.iter().map(|(dl, _)| dl).sum::<f64>() / pairs.len() as f64
        } else {
            0.0
        }
    };

    // Upload: volume-weighted average
    let upload = {
        let pairs: Vec<(f64, u64)> = successful
            .iter()
            .filter_map(|p| p.upload_mbps.map(|ul| (ul, p.upload_bytes)))
            .filter(|(ul, _)| *ul > 0.0)
            .collect();
        let total_bytes: u64 = pairs.iter().map(|(_, b)| *b).sum();
        if total_bytes > 0 {
            pairs
                .iter()
                .map(|(ul, b)| ul * *b as f64)
                .sum::<f64>()
                / total_bytes as f64
        } else if !pairs.is_empty() {
            pairs.iter().map(|(ul, _)| ul).sum::<f64>() / pairs.len() as f64
        } else {
            0.0
        }
    };

    // Packet loss from Cloudflare (only provider that measures it)
    let packet_loss = successful
        .iter()
        .find(|p| p.provider == "Cloudflare")
        .and_then(|p| p.packet_loss_pct);

    (ping, jitter, download, upload, packet_loss)
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

    let (ping, jitter, download, upload, packet_loss) = aggregate(&providers);
    let duration = start.elapsed().as_secs_f64();

    SpeedTestResult {
        ping_ms: ping,
        jitter_ms: jitter,
        download_mbps: download,
        upload_mbps: upload,
        packet_loss_pct: packet_loss,
        providers,
        duration_s: duration,
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
