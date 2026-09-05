use clap::Parser;
use nd_300::cli::{SpeedQXCli, SpeedQXCommand};
use nd_300::speedtest::display::{render_results, SpeedQXDisplay};
use nd_300::speedtest::{
    format_mbps, Phase, ProviderCompleteCallback, ProviderSet, SpeedTestConfig,
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
fn plan_counts(fast: bool, skip_msak: bool, no_mlab: bool) -> (u32, u32) {
    let primary = if skip_msak { 1 } else { 2 };
    let count =
        primary * if fast { 1 } else { 2 } + if no_mlab { 0 } else { 1 } + if fast { 0 } else { 4 };
    (count, count * 3 + 1 - if fast { 0 } else { 3 })
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
    let fast = !cli.deep;

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
        max_bytes: cli.max_bytes,
        mlab_consent: cli.accept_mlab,
        ..SpeedTestConfig::default()
    };

    let (provider_count, total_steps) =
        plan_counts(fast, !cli.accept_mlab || cli.skip_msak, !cli.accept_mlab);

    if !json_mode {
        SpeedQXDisplay::new(use_ascii, use_colors, json_mode).print_header();
        println!(
            "  {} · up to {} seconds · up to {} payload bytes",
            if fast { "Quick" } else { "Deep" },
            if fast { 90 } else { 300 },
            config
                .max_bytes
                .unwrap_or(if fast { 5_000_000_000 } else { 20_000_000_000 })
        );
        if cli.accept_mlab {
            println!("  M-Lab publishes measurement results and your IP address.");
        } else {
            println!("  Cloudflare is the only primary source. Add --accept-mlab to consent to M-Lab data publication.");
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
    let cancellation = config.cancel.clone();
    let running = nd_300::speedtest::run(
        config,
        move |phase, progress| {
            if let Ok(mut s) = state_clone.lock() {
                s.handle_phase(phase, progress);
            }
        },
        Some(on_complete),
    );
    tokio::pin!(running);
    let result = tokio::select! {
        result = &mut running => result,
        _ = tokio::signal::ctrl_c() => {
            cancellation.store(true, std::sync::atomic::Ordering::Relaxed);
            running.await
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

    if cancellation.load(std::sync::atomic::Ordering::Relaxed) {
        std::process::exit(130);
    }
    // Honest exit code: non-zero when no provider produced positive throughput
    // in either direction (mirrors diagnostics/speed.rs).
    let measured = result
        .measurement
        .as_ref()
        .is_some_and(|m| m.download.sustained_mbps.is_some() || m.upload.sustained_mbps.is_some());
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
