use indicatif::{ProgressBar, ProgressStyle};

use crate::config::Config;
use crate::speedtest::{self, Phase, SpeedTestConfig, SpeedTestResult, TestDuration};

use super::DiagnosticResult;

pub async fn check(config: &Config) -> (DiagnosticResult, Option<SpeedTestResult>) {
    let st_config = SpeedTestConfig {
        duration: TestDuration::Seconds(config.speed_duration),
        fastcom_duration: TestDuration::Auto,
        latency_probes: 10,
        provider_set: speedtest::ProviderSet::Diagnostic,
        use_colors: config.use_colors,
    };

    // Single progress bar covering all phases
    let pb = create_progress_bar(config);
    let pb_clone = pb.clone();

    let result = speedtest::run(st_config, move |phase, progress| {
        update_progress(&pb_clone, phase, progress);
    }, None)
    .await;

    pb.finish_and_clear();

    // Convert to DiagnosticResult
    let summary = format_speed_summary(&result);
    let status = determine_speed_status(&result);

    let diag = match status {
        SpeedStatus::Good => DiagnosticResult::ok("Speed", summary),
        SpeedStatus::Warning(note) => {
            DiagnosticResult::warn("Speed", format!("{}\n{}", summary, note))
        }
        SpeedStatus::Poor(note) => {
            DiagnosticResult::warn("Speed", format!("{}\n{}", summary, note))
        }
    };

    (diag, Some(result))
}

fn create_progress_bar(config: &Config) -> ProgressBar {
    let pb = ProgressBar::new(100);
    let template = if config.use_colors {
        "  {spinner:.cyan} Speed test  [{bar:30.cyan/dim}] {pos}% {msg}"
    } else {
        "  {spinner} Speed test  [{bar:30}] {pos}% {msg}"
    };
    pb.set_style(
        ProgressStyle::default_bar()
            .template(template)
            .unwrap_or_else(|_| ProgressStyle::default_bar())
            .progress_chars("━╸─"),
    );
    pb.set_message("Starting...");
    pb
}

fn update_progress(pb: &ProgressBar, phase: Phase, progress: f64) {
    // Map phase + progress to overall percentage:
    // CF latency:    0-10%
    // CF download:  10-30%
    // CF upload:    30-50%
    // NDT7 discovery: 50-55%
    // NDT7 download: 55-75%
    // NDT7 upload:  75-100%
    // nd300 only runs Diagnostic provider set (CF + NDT7), but we handle
    // all Phase variants for completeness — LS/FC phases won't fire.
    let (start, range, msg) = match phase {
        Phase::CfLatency => (0.0, 10.0, "CF latency..."),
        Phase::CfDownload => (10.0, 20.0, "CF download..."),
        Phase::CfUpload => (30.0, 20.0, "CF upload..."),
        Phase::Ndt7Discovery => (50.0, 5.0, "NDT7 discovery..."),
        Phase::Ndt7Download => (55.0, 20.0, "NDT7 download..."),
        Phase::Ndt7Upload => (75.0, 25.0, "NDT7 upload..."),
        Phase::LsDiscovery => (85.0, 2.0, "LS discovery..."),
        Phase::LsDownload => (87.0, 5.0, "LS download..."),
        Phase::LsUpload => (92.0, 5.0, "LS upload..."),
        Phase::FcDiscovery => (97.0, 1.0, "FC discovery..."),
        Phase::FcDownload => (98.0, 1.0, "FC download..."),
        Phase::FcUpload => (99.0, 1.0, "FC upload..."),
        Phase::Computing => (100.0, 0.0, "Computing..."),
    };

    let overall = (start + range * progress.clamp(0.0, 1.0)).min(100.0) as u64;
    pb.set_position(overall);
    pb.set_message(msg);
}

fn format_speed_summary(result: &SpeedTestResult) -> String {
    let dl = speedtest::format_mbps(result.download_mbps);
    let ul = speedtest::format_mbps(result.upload_mbps);

    match result.ping_ms {
        Some(ping) => format!("{} down / {} up ({}ms)", dl, ul, ping.round() as u64),
        None => format!("{} down / {} up", dl, ul),
    }
}

enum SpeedStatus {
    Good,
    Warning(String),
    Poor(String),
}

fn determine_speed_status(result: &SpeedTestResult) -> SpeedStatus {
    let download = result.download_mbps;
    let upload = result.upload_mbps;

    if download < 5.0 {
        SpeedStatus::Poor("Download too slow for most activities".to_string())
    } else if upload < 1.0 {
        SpeedStatus::Poor("Upload too slow for video calls".to_string())
    } else if download < 25.0 {
        SpeedStatus::Warning("May struggle with HD streaming".to_string())
    } else if upload < 5.0 {
        SpeedStatus::Warning("Upload may be slow for video calls".to_string())
    } else {
        SpeedStatus::Good
    }
}
