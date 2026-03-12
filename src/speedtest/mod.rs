pub mod cloudflare;
pub mod display;
pub mod ndt7;

use serde::Serialize;
use std::sync::Arc;
use std::time::Instant;

/// Test duration configuration
#[derive(Debug, Clone)]
pub enum TestDuration {
    /// Fixed duration per provider in seconds
    Seconds(u64),
    /// Let providers use their natural duration
    Auto,
}

/// Configuration for the speed test orchestrator
#[derive(Debug, Clone)]
pub struct SpeedTestConfig {
    pub duration: TestDuration,
    pub latency_probes: u32,
    pub run_cloudflare: bool,
    pub run_ndt7: bool,
    pub use_colors: bool,
}

impl Default for SpeedTestConfig {
    fn default() -> Self {
        Self {
            duration: TestDuration::Seconds(30),
            latency_probes: 20,
            run_cloudflare: true,
            run_ndt7: true,
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

fn aggregate(providers: &[ProviderResult]) -> (Option<f64>, Option<f64>, f64, f64, Option<f64>) {
    let successful: Vec<&ProviderResult> = providers.iter().filter(|p| p.error.is_none()).collect();

    if successful.is_empty() {
        return (None, None, 0.0, 0.0, None);
    }

    let pings: Vec<f64> = successful
        .iter()
        .filter_map(|p| p.ping_ms)
        .filter(|p| *p > 0.0)
        .collect();

    let jitters: Vec<f64> = successful
        .iter()
        .filter_map(|p| p.jitter_ms)
        .filter(|j| *j > 0.0)
        .collect();

    let downloads: Vec<f64> = successful
        .iter()
        .filter_map(|p| p.download_mbps)
        .filter(|d| *d > 0.0)
        .collect();

    let uploads: Vec<f64> = successful
        .iter()
        .filter_map(|p| p.upload_mbps)
        .filter(|u| *u > 0.0)
        .collect();

    let ping = if pings.is_empty() {
        None
    } else {
        Some(pings.iter().sum::<f64>() / pings.len() as f64)
    };

    let jitter = if jitters.is_empty() {
        None
    } else {
        Some(jitters.iter().sum::<f64>() / jitters.len() as f64)
    };

    let download = if downloads.is_empty() {
        0.0
    } else {
        downloads.iter().sum::<f64>() / downloads.len() as f64
    };

    let upload = if uploads.is_empty() {
        0.0
    } else {
        uploads.iter().sum::<f64>() / uploads.len() as f64
    };

    // Packet loss only from Cloudflare
    let packet_loss = successful
        .iter()
        .find(|p| p.provider == "Cloudflare")
        .and_then(|p| p.packet_loss_pct);

    (ping, jitter, download, upload, packet_loss)
}

/// Run the speed test with the given configuration and progress callback.
pub async fn run<F>(config: SpeedTestConfig, progress: F) -> SpeedTestResult
where
    F: Fn(Phase, f64) + Send + Sync + 'static,
{
    let start = Instant::now();
    let mut providers = Vec::new();
    let progress = Arc::new(progress);

    if config.run_cloudflare {
        let pg = progress.clone();
        let cf_result = cloudflare::run(&config, move |phase, p| pg(phase, p)).await;
        providers.push(cf_result);
    }

    if config.run_ndt7 {
        let pg = progress.clone();
        let ndt_result = ndt7::run(&config, move |phase, p| pg(phase, p)).await;
        providers.push(ndt_result);
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
