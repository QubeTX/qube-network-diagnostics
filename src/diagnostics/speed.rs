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

    let result = speedtest::run(
        st_config,
        move |phase, progress| {
            update_progress(&pb_clone, phase, progress);
        },
        None,
    )
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
        // No provider returned a usable measurement — report an honest failure
        // (exit 2) rather than a misleading "too slow" warning built on a
        // 0.00 Mbps aggregate. The standalone summary intentionally omits the
        // "0.00 Mbps down / up" line since nothing was actually measured.
        SpeedStatus::Failed => {
            DiagnosticResult::fail("Speed", "Speed test failed — no provider returned a result")
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
    /// Every provider errored (or returned zero throughput) — nothing was
    /// actually measured. Distinct from a genuinely slow-but-working link.
    Failed,
}

fn determine_speed_status(result: &SpeedTestResult) -> SpeedStatus {
    // A run counts as "measured" only if at least one provider succeeded
    // (no error) AND reported positive throughput in either direction. The
    // `> 0.0` guard is critical: a real-but-slow link (e.g. 0.3 Mbps) has a
    // successful provider with positive throughput, so it stays Poor → Warn.
    // Only an all-errored / all-zero run is Failed → Fail (exit 2).
    let measured = result.providers.iter().any(|p| {
        p.error.is_none()
            && (p.download_mbps.unwrap_or(0.0) > 0.0 || p.upload_mbps.unwrap_or(0.0) > 0.0)
    });
    if !measured {
        return SpeedStatus::Failed;
    }

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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diagnostics::DiagnosticStatus;
    use crate::speedtest::ProviderResult;

    /// Build a ProviderResult with the given outcome. `error` Some marks a
    /// failed provider; `dl`/`ul` are Option throughput values.
    fn provider(error: Option<&str>, dl: Option<f64>, ul: Option<f64>) -> ProviderResult {
        ProviderResult {
            provider: "Test".to_string(),
            server: "test-server".to_string(),
            location: None,
            ping_ms: None,
            jitter_ms: None,
            download_mbps: dl,
            upload_mbps: ul,
            download_bytes: 0,
            upload_bytes: 0,
            download_duration_s: 0.0,
            upload_duration_s: 0.0,
            packet_loss_pct: None,
            error: error.map(|e| e.to_string()),
            bandwidth_samples: None,
        }
    }

    fn result(
        providers: Vec<ProviderResult>,
        download_mbps: f64,
        upload_mbps: f64,
    ) -> SpeedTestResult {
        SpeedTestResult {
            ping_ms: None,
            jitter_ms: None,
            download_mbps,
            upload_mbps,
            packet_loss_pct: None,
            providers,
            duration_s: 0.0,
            stability: None,
            provider_divergence: None,
            confidence_intervals: None,
            merge_exclusions: Vec::new(),
        }
    }

    #[test]
    fn all_errored_providers_report_failed() {
        // Every provider errored; throughput None and aggregate 0.0/0.0.
        let res = result(
            vec![
                provider(Some("connect failed"), None, None),
                provider(Some("discovery failed"), None, None),
            ],
            0.0,
            0.0,
        );
        assert!(matches!(determine_speed_status(&res), SpeedStatus::Failed));
    }

    #[test]
    fn all_zero_throughput_reports_failed() {
        // Providers "succeeded" (no error) but reported zero throughput —
        // still nothing measured, so Failed.
        let res = result(vec![provider(None, Some(0.0), Some(0.0))], 0.0, 0.0);
        assert!(matches!(determine_speed_status(&res), SpeedStatus::Failed));
    }

    #[test]
    fn slow_but_working_link_is_poor_not_failed() {
        // A real link that's genuinely slow: a successful provider with a
        // small positive download. Must be Poor (→ Warn), NOT Failed. This
        // guards against the false-Fail on a working-but-slow connection.
        let res = result(vec![provider(None, Some(0.3), None)], 0.3, 0.0);
        match determine_speed_status(&res) {
            SpeedStatus::Poor(_) => {}
            other => panic!(
                "expected Poor for a slow-but-working link, got {:?}",
                std::mem::discriminant(&other)
            ),
        }
    }

    #[test]
    fn check_maps_failed_to_fail_status() {
        // Verify the enum → DiagnosticResult mapping: an all-errored run
        // becomes a Fail-status diagnostic (which aggregates to exit 2),
        // with an honest standalone summary (no "0.00 Mbps" line).
        let res = result(vec![provider(Some("connect failed"), None, None)], 0.0, 0.0);
        let status = determine_speed_status(&res);
        assert!(matches!(status, SpeedStatus::Failed));

        // Mirror the mapping in `check()` for the Failed branch.
        let diag = match status {
            SpeedStatus::Failed => {
                DiagnosticResult::fail("Speed", "Speed test failed — no provider returned a result")
            }
            _ => unreachable!("constructed an all-errored result"),
        };
        assert_eq!(diag.status, DiagnosticStatus::Fail);
        assert!(!diag.summary.contains("0.00"));
    }

    #[test]
    fn good_link_with_one_failed_provider_is_not_failed() {
        // Mixed: one provider failed, one succeeded with good throughput.
        // The successful provider means the run was measured → not Failed.
        let res = result(
            vec![
                provider(Some("connect failed"), None, None),
                provider(None, Some(120.0), Some(40.0)),
            ],
            120.0,
            40.0,
        );
        assert!(matches!(determine_speed_status(&res), SpeedStatus::Good));
    }
}
