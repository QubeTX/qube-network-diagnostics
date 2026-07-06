use clap::Parser;
use nd_300::cli::{SpeedQXCli, SpeedQXCommand};
use nd_300::speedtest::display::{render_results, SpeedQXDisplay};
use nd_300::speedtest::{
    format_mbps, Phase, ProviderCompleteCallback, ProviderSet, SpeedTestConfig, TestDuration,
};
use std::sync::{Arc, Mutex};

/// Tracks which phase is currently active so the callback can manage
/// transitions. Steps are a running counter (advanced on every new phase)
/// rather than fixed indices, so the display adapts to any provider set
/// (FAST / FULL / skip flags) and the variable per-provider phase counts.
struct DisplayState {
    display: SpeedQXDisplay,
    current_phase: Option<Phase>,
    current_bar: Option<indicatif::ProgressBar>,
    /// Running step counter (1-based), advanced when a new phase begins.
    current_step: u32,
    total_steps: u32,
    /// How many providers run this session (for the "Provider X/N" banner).
    provider_count: u32,
    /// 1-based index of the provider currently being displayed.
    provider_index: u32,
    current_provider: Option<&'static str>,
    use_colors: bool,
    use_ascii: bool,
    json_mode: bool,
}

/// Human-facing provider name for a phase (drives the transition banner).
fn provider_name_for_phase(phase: Phase) -> &'static str {
    match phase {
        Phase::CfLatency | Phase::CfDownload | Phase::CfUpload => "Cloudflare",
        Phase::Ndt7Discovery | Phase::Ndt7Download | Phase::Ndt7Upload => "M-Lab NDT7",
        Phase::MsakDiscovery | Phase::MsakDownload | Phase::MsakUpload => "M-Lab MSAK",
        Phase::LsDiscovery | Phase::LsDownload | Phase::LsUpload => "LibreSpeed",
        Phase::FcDiscovery | Phase::FcDownload | Phase::FcUpload => "fast.com (Netflix)",
        Phase::CfyLatency | Phase::CfyDownload => "CacheFly",
        Phase::VultrDiscovery | Phase::VultrLatency | Phase::VultrDownload => "Vultr",
        Phase::AnqDiscovery | Phase::AnqDownload | Phase::AnqUpload => "Apple networkQuality",
        Phase::Computing => "Computing",
    }
}

impl DisplayState {
    fn label_for_phase(&self, phase: Phase) -> &'static str {
        match phase {
            Phase::CfLatency => "Measuring latency (Cloudflare)",
            Phase::CfDownload => "Download (Cloudflare)",
            Phase::CfUpload => "Upload (Cloudflare)",
            Phase::Ndt7Discovery => "Finding nearest M-Lab server",
            Phase::Ndt7Download => "Download (M-Lab NDT7)",
            Phase::Ndt7Upload => "Upload (M-Lab NDT7)",
            Phase::MsakDiscovery => "Finding nearest M-Lab MSAK server",
            Phase::MsakDownload => "Download (M-Lab MSAK, multi-stream)",
            Phase::MsakUpload => "Upload (M-Lab MSAK, multi-stream)",
            Phase::LsDiscovery => "Finding nearest LibreSpeed server",
            Phase::LsDownload => "Download (LibreSpeed)",
            Phase::LsUpload => "Upload (LibreSpeed)",
            Phase::FcDiscovery => "Connecting to Netflix CDN",
            Phase::FcDownload => "Download (fast.com)",
            Phase::FcUpload => "Upload (fast.com)",
            Phase::CfyLatency => "Measuring latency (CacheFly)",
            Phase::CfyDownload => "Download (CacheFly)",
            Phase::VultrDiscovery => "Selecting nearest Vultr PoP",
            Phase::VultrLatency => "Measuring latency (Vultr)",
            Phase::VultrDownload => "Download (Vultr)",
            Phase::AnqDiscovery => "Connecting to Apple edge",
            Phase::AnqDownload => "Download (Apple networkQuality)",
            Phase::AnqUpload => "Upload (Apple networkQuality)",
            Phase::Computing => "Results computed",
        }
    }

    fn is_progress_phase(&self, phase: Phase) -> bool {
        matches!(
            phase,
            Phase::CfDownload
                | Phase::CfUpload
                | Phase::Ndt7Download
                | Phase::Ndt7Upload
                | Phase::MsakDownload
                | Phase::MsakUpload
                | Phase::LsDownload
                | Phase::LsUpload
                | Phase::FcDownload
                | Phase::FcUpload
                | Phase::CfyDownload
                | Phase::VultrDownload
                | Phase::AnqDownload
                | Phase::AnqUpload
        )
    }

    fn handle_phase(&mut self, phase: Phase, progress: f64) {
        // Phase transition: finish the previous bar and open a new one.
        if self.current_phase != Some(phase) {
            // Don't re-enter a phase that was already finished (a spurious
            // trailing progress >= 1.0 for a completed phase).
            if self.current_bar.is_none() && self.current_phase.is_none() && progress >= 1.0 {
                return;
            }

            self.finish_current();

            // Provider transition banner when entering a new provider.
            let name = provider_name_for_phase(phase);
            if phase != Phase::Computing && self.current_provider != Some(name) {
                self.current_provider = Some(name);
                self.provider_index += 1;
                if !self.json_mode {
                    let sep = if self.use_ascii { "-" } else { "\u{2500}" };
                    let banner = format!(
                        "  {} Provider {}/{}: {} {}",
                        sep.repeat(2),
                        self.provider_index,
                        self.provider_count,
                        name,
                        sep.repeat(30usize.saturating_sub(name.len()))
                    );
                    if self.use_colors {
                        println!("{}", owo_colors::OwoColorize::dimmed(&banner));
                    } else {
                        println!("{}", banner);
                    }
                }
            }

            self.current_step = (self.current_step + 1).min(self.total_steps);
            self.current_phase = Some(phase);

            let label = self.label_for_phase(phase);
            if self.is_progress_phase(phase) {
                let bar =
                    self.display
                        .create_progress_bar(self.current_step, self.total_steps, label);
                self.current_bar = Some(bar);
            } else {
                let spinner =
                    self.display
                        .create_spinner(self.current_step, self.total_steps, label);
                self.current_bar = Some(spinner);
            }
        }

        // Update the active progress bar.
        if let Some(ref bar) = self.current_bar {
            if self.is_progress_phase(phase) {
                let pct = (progress * 100.0).min(100.0) as u64;
                bar.set_position(pct);
            }
        }

        // Finish immediately once the phase reports completion.
        if progress >= 1.0 {
            self.finish_current();
        }
    }

    fn finish_current(&mut self) {
        if let Some(bar) = self.current_bar.take() {
            bar.finish_and_clear();
        }
        if let Some(phase) = self.current_phase.take() {
            let label = self.label_for_phase(phase);
            self.display
                .finish_step(self.current_step, self.total_steps, label);
        }
    }
}

/// Compute the running-provider count and the total number of phase steps for
/// the header + the [step/total] display, mirroring the plan `speedtest::run`
/// executes. Phase counts per provider: Cloudflare 3, NDT7 3, MSAK 3,
/// LibreSpeed 3, fast.com 3, CacheFly 2 (download-only), Vultr 3, Apple 3,
/// plus 1 for the final Computing step.
fn plan_counts(fast: bool, skip_msak: bool, skip_apple: bool) -> (u32, u32) {
    let mut provider_count: u32 = 2; // Cloudflare + NDT7
    let mut phase_steps: u32 = 6; // 3 + 3

    // MSAK: FAST always runs it; FULL runs it unless skipped.
    if fast || !skip_msak {
        provider_count += 1;
        phase_steps += 3;
    }

    if !fast {
        // FULL adds LibreSpeed (3), fast.com (3), CacheFly (2), Vultr (3).
        provider_count += 4;
        phase_steps += 3 + 3 + 2 + 3;
        if !skip_apple {
            provider_count += 1;
            phase_steps += 3;
        }
    }

    (provider_count, phase_steps + 1) // + Computing
}

#[tokio::main]
async fn main() {
    let cli = SpeedQXCli::parse();

    #[cfg(windows)]
    enable_utf8_console();

    // Subcommand form takes precedence over the legacy --update flag.
    if let Some(cmd) = cli.command.clone() {
        match cmd {
            SpeedQXCommand::Update => {
                let mut config = nd_300::config::Config::new().with_colors(!cli.no_color);
                if cli.json {
                    config = config.with_json();
                }
                let exit_code = nd_300::actions::update::run(&config).await;
                std::process::exit(exit_code);
            }
        }
    }

    // Legacy flag form: `speedqx --update`.
    if cli.update {
        let mut config = nd_300::config::Config::new().with_colors(!cli.no_color);
        if cli.json {
            config = config.with_json();
        }
        let exit_code = nd_300::actions::update::run(&config).await;
        std::process::exit(exit_code);
    }

    let use_ascii = cli.ascii;
    let use_colors = !cli.no_color;
    let json_mode = cli.json;
    let fast = cli.fast;

    let provider_set = if fast {
        ProviderSet::Fast
    } else {
        ProviderSet::All
    };

    let config = SpeedTestConfig {
        duration: cli.duration,
        fastcom_duration: cli.fastcom_duration,
        latency_probes: cli.latency_probes,
        provider_set,
        // In FAST mode the fixed subset (CF + NDT7 + MSAK) always runs; the
        // skip flags only apply to the FULL run.
        msak_enabled: !cli.skip_msak,
        apple_enabled: !cli.skip_apple,
        use_colors,
    };

    let (provider_count, total_steps) = plan_counts(fast, cli.skip_msak, cli.skip_apple);

    // Outer wall-clock cap. The providers run sequentially; each fixed-duration
    // provider does `duration` per direction plus a dense latency phase, and
    // fast.com uses `fastcom_duration`. FAST providers are additionally capped
    // at 25 s each with early stopping. This generous ceiling only trips on a
    // genuinely stuck provider.
    let cap_dur_secs = match &config.duration {
        TestDuration::Seconds(s) => *s,
        TestDuration::Auto => 15,
    };
    let cap_fc_secs = match &config.fastcom_duration {
        TestDuration::Seconds(s) => *s,
        TestDuration::Auto => 15,
    };
    let outer_cap = std::time::Duration::from_secs(
        (provider_count as u64) * (2 * cap_dur_secs + 25) + 2 * cap_fc_secs + 90,
    )
    .max(std::time::Duration::from_secs(120));

    // Print header with an estimated time.
    if !json_mode {
        let display = SpeedQXDisplay::new(use_ascii, use_colors, json_mode);
        display.print_header();

        let per_dir_secs = if fast { 8 } else { cap_dur_secs };
        let total_est = (provider_count as u64) * (2 * per_dir_secs + 6);
        let mins = total_est / 60;
        let secs = total_est % 60;
        let mode = if fast { "FAST" } else { "FULL" };

        if use_colors {
            println!(
                "  {}",
                owo_colors::OwoColorize::dimmed(&format!(
                    "Estimated test time: ~{}:{:02} ({} mode, {} providers, {}s/direction)",
                    mins, secs, mode, provider_count, per_dir_secs
                ))
            );
        } else {
            println!(
                "  Estimated test time: ~{}:{:02} ({} mode, {} providers, {}s/direction)",
                mins, secs, mode, provider_count, per_dir_secs
            );
        }
        println!();
    }

    let state = Arc::new(Mutex::new(DisplayState {
        display: SpeedQXDisplay::new(use_ascii, use_colors, json_mode),
        current_phase: None,
        current_bar: None,
        current_step: 0,
        total_steps,
        provider_count,
        provider_index: 0,
        current_provider: None,
        use_colors,
        use_ascii,
        json_mode,
    }));

    // Provider completion callback — prints a summary after each provider.
    let summary_colors = use_colors;
    let summary_ascii = use_ascii;
    let summary_json = json_mode;
    let on_complete: ProviderCompleteCallback = Arc::new(move |result| {
        if summary_json {
            return;
        }

        let sep = if summary_ascii {
            "---"
        } else {
            "\u{2500}\u{2500}\u{2500}"
        };

        let dl = result
            .download_mbps
            .map(|d| format!("{} \u{2193}", format_mbps(d)))
            .unwrap_or_else(|| "N/A \u{2193}".to_string());
        let ul = result
            .upload_mbps
            .map(|u| format!("{} \u{2191}", format_mbps(u)))
            .unwrap_or_else(|| "N/A \u{2191}".to_string());
        let ping = result
            .ping_ms
            .map(|p| format!(" ({}ms)", p.round() as u64))
            .unwrap_or_default();

        if let Some(ref err) = result.error {
            if summary_colors {
                println!(
                    "  {} {}: {} {}",
                    sep,
                    result.provider,
                    owo_colors::OwoColorize::red(&err.as_str()),
                    sep
                );
            } else {
                println!("  {} {}: {} {}", sep, result.provider, err, sep);
            }
        } else if summary_colors {
            println!(
                "  {} {}: {} / {}{} {}",
                sep,
                owo_colors::OwoColorize::bold(&result.provider.as_str()),
                owo_colors::OwoColorize::green(&dl.as_str()),
                owo_colors::OwoColorize::cyan(&ul.as_str()),
                owo_colors::OwoColorize::dimmed(&ping.as_str()),
                sep
            );
        } else {
            println!(
                "  {} {}: {} / {}{} {}",
                sep, result.provider, dl, ul, ping, sep
            );
        }
        println!();
    });

    let state_clone = state.clone();
    let result = match tokio::time::timeout(
        outer_cap,
        nd_300::speedtest::run(
            config,
            move |phase, progress| {
                if let Ok(mut s) = state_clone.lock() {
                    s.handle_phase(phase, progress);
                }
            },
            Some(on_complete),
        ),
    )
    .await
    {
        Ok(r) => r,
        Err(_) => {
            if json_mode {
                println!(
                    "{{\"error\":\"timeout\",\"timed_out\":true,\"timeout_secs\":{}}}",
                    outer_cap.as_secs()
                );
            } else {
                eprintln!();
                eprintln!(
                    "Speed test timed out after {}s — a provider appears to be stuck or the \
                     network is severely degraded. Try again, or use --duration to shorten the test.",
                    outer_cap.as_secs()
                );
            }
            std::process::exit(2);
        }
    };

    if json_mode {
        match serde_json::to_string_pretty(&result) {
            Ok(json) => println!("{}", json),
            Err(e) => {
                eprintln!("Error serializing results: {}", e);
                std::process::exit(1);
            }
        }
    } else {
        println!();
        print!("{}", render_results(&result, use_ascii, use_colors));
    }

    // Honest exit code: non-zero when no provider produced positive throughput
    // in either direction (mirrors diagnostics/speed.rs).
    let measured = result.providers.iter().any(|p| {
        p.error.is_none()
            && (p.download_mbps.unwrap_or(0.0) > 0.0 || p.upload_mbps.unwrap_or(0.0) > 0.0)
    });
    if !measured {
        std::process::exit(2);
    }
}

#[cfg(windows)]
fn enable_utf8_console() {
    use std::io::IsTerminal;
    if std::io::stdout().is_terminal() {
        unsafe {
            winapi::um::wincon::SetConsoleOutputCP(65001);
        }
    }
}
