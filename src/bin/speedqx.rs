use clap::Parser;
use nd_300::cli::{SpeedQXCli, SpeedQXCommand};
use nd_300::speedtest::display::{render_results, SpeedQXDisplay};
use nd_300::speedtest::{
    format_mbps, Phase, ProviderCompleteCallback, SpeedTestConfig, TestDuration,
};
use std::sync::{Arc, Mutex};

/// Tracks which phase is currently active so the callback can manage transitions.
struct DisplayState {
    display: SpeedQXDisplay,
    current_phase: Option<Phase>,
    current_bar: Option<indicatif::ProgressBar>,
    total_steps: u32,
    use_colors: bool,
    use_ascii: bool,
    json_mode: bool,
    last_provider_num: u32,
}

impl DisplayState {
    fn step_for_phase(&self, phase: Phase) -> u32 {
        match phase {
            Phase::CfLatency => 1,
            Phase::CfDownload => 2,
            Phase::CfUpload => 3,
            Phase::Ndt7Discovery => 4,
            Phase::Ndt7Download => 5,
            Phase::Ndt7Upload => 6,
            Phase::LsDiscovery => 7,
            Phase::LsDownload => 8,
            Phase::LsUpload => 9,
            Phase::FcDiscovery => 10,
            Phase::FcDownload => 11,
            Phase::FcUpload => 12,
            Phase::Computing => 13,
        }
    }

    fn label_for_phase(&self, phase: Phase) -> &'static str {
        match phase {
            Phase::CfLatency => "Measuring latency (Cloudflare)",
            Phase::CfDownload => "Download (Cloudflare)",
            Phase::CfUpload => "Upload (Cloudflare)",
            Phase::Ndt7Discovery => "Finding nearest M-Lab server",
            Phase::Ndt7Download => "Download (M-Lab NDT7)",
            Phase::Ndt7Upload => "Upload (M-Lab NDT7)",
            Phase::LsDiscovery => "Finding nearest LibreSpeed server",
            Phase::LsDownload => "Download (LibreSpeed)",
            Phase::LsUpload => "Upload (LibreSpeed)",
            Phase::FcDiscovery => "Connecting to Netflix CDN",
            Phase::FcDownload => "Download (fast.com)",
            Phase::FcUpload => "Upload (fast.com)",
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
                | Phase::LsDownload
                | Phase::LsUpload
                | Phase::FcDownload
                | Phase::FcUpload
        )
    }

    /// Get the provider number (1-4) for a given phase.
    fn provider_num_for_phase(&self, phase: Phase) -> u32 {
        match phase {
            Phase::CfLatency | Phase::CfDownload | Phase::CfUpload => 1,
            Phase::Ndt7Discovery | Phase::Ndt7Download | Phase::Ndt7Upload => 2,
            Phase::LsDiscovery | Phase::LsDownload | Phase::LsUpload => 3,
            Phase::FcDiscovery | Phase::FcDownload | Phase::FcUpload => 4,
            Phase::Computing => 5,
        }
    }

    fn provider_name_for_num(&self, num: u32) -> &'static str {
        match num {
            1 => "Cloudflare",
            2 => "M-Lab NDT7",
            3 => "LibreSpeed",
            4 => "fast.com (Netflix)",
            _ => "Computing",
        }
    }

    fn handle_phase(&mut self, phase: Phase, progress: f64) {
        let step = self.step_for_phase(phase);

        // Phase transition: finish previous bar and print completion line
        if self.current_phase != Some(phase) {
            // Don't re-enter a phase that was already finished (prevents duplicate lines)
            if self.current_bar.is_none() && self.current_phase.is_none() && progress >= 1.0 {
                return;
            }

            self.finish_current();

            // Print provider transition banner when entering a new provider
            let provider_num = self.provider_num_for_phase(phase);
            if provider_num != self.last_provider_num && provider_num <= 4 && !self.json_mode {
                self.last_provider_num = provider_num;
                let name = self.provider_name_for_num(provider_num);
                let sep = if self.use_ascii { "-" } else { "\u{2500}" };
                let banner = format!(
                    "  {} Provider {}/4: {} {}",
                    sep.repeat(2),
                    provider_num,
                    name,
                    sep.repeat(30usize.saturating_sub(name.len()))
                );
                if self.use_colors {
                    println!("{}", owo_colors::OwoColorize::dimmed(&banner));
                } else {
                    println!("{}", banner);
                }
            }

            self.current_phase = Some(phase);

            let label = self.label_for_phase(phase);

            if self.is_progress_phase(phase) {
                let bar = self
                    .display
                    .create_progress_bar(step, self.total_steps, label);
                self.current_bar = Some(bar);
            } else {
                let spinner = self.display.create_spinner(step, self.total_steps, label);
                self.current_bar = Some(spinner);
            }
        }

        // Update progress on active bar
        if let Some(ref bar) = self.current_bar {
            if self.is_progress_phase(phase) {
                let pct = (progress * 100.0).min(100.0) as u64;
                bar.set_position(pct);
            }
        }

        // If progress is 1.0, finish this phase immediately
        if progress >= 1.0 {
            self.finish_current();
        }
    }

    fn finish_current(&mut self) {
        if let Some(bar) = self.current_bar.take() {
            bar.finish_and_clear();
        }
        if let Some(phase) = self.current_phase.take() {
            let step = self.step_for_phase(phase);
            let label = self.label_for_phase(phase);
            self.display.finish_step(step, self.total_steps, label);
        }
    }
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

    let config = SpeedTestConfig {
        duration: cli.duration,
        fastcom_duration: cli.fastcom_duration,
        latency_probes: cli.latency_probes,
        provider_set: nd_300::speedtest::ProviderSet::All,
        use_colors,
    };

    // Outer wall-clock cap (M1). nd300 already bounds its diagnostic run; speedqx
    // had none, so a CDN that wedged past every per-request timeout could leave
    // the whole test hanging. The four providers run sequentially — CF/NDT7/LS
    // each do `duration` per direction (×2), fast.com does `fastcom_duration` per
    // direction — so the legitimate ceiling is ~`2*(3*duration) + 2*fastcom`.
    // Generous discovery/latency headroom plus a floor keep a healthy run well
    // inside the cap; only a genuinely stuck provider trips it. Computed here
    // before `config` is moved into `speedtest::run` below.
    let cap_dur_secs = match &config.duration {
        TestDuration::Seconds(s) => *s,
        TestDuration::Auto => 15,
    };
    let cap_fc_secs = match &config.fastcom_duration {
        TestDuration::Seconds(s) => *s,
        TestDuration::Auto => 15,
    };
    let outer_cap = std::time::Duration::from_secs(
        // 3 providers × 2 directions × duration + fast.com × 2 directions, plus
        // a 2× safety multiple for retries/per-request floors, plus 60s headroom.
        (2 * (3 * cap_dur_secs) + 2 * cap_fc_secs) * 2 + 60,
    )
    .max(std::time::Duration::from_secs(120));

    let total_steps: u32 = 13; // CF(3) + NDT7(3) + LS(3) + FC(3) + Computing(1)

    // Print header with estimated time
    if !json_mode {
        let display = SpeedQXDisplay::new(use_ascii, use_colors, json_mode);
        display.print_header();

        // Estimate total time based on duration config
        let per_dir_secs = match &config.duration {
            TestDuration::Seconds(s) => *s,
            TestDuration::Auto => 15,
        };
        let fc_secs = match &config.fastcom_duration {
            TestDuration::Seconds(s) => *s * 2,
            TestDuration::Auto => 25, // ~15s DL + ~10s UL
        };
        let total_est = per_dir_secs * 2 * 3 + fc_secs; // 3 providers * 2 dirs + FC
        let mins = total_est / 60;
        let secs = total_est % 60;

        if use_colors {
            println!(
                "  {}",
                owo_colors::OwoColorize::dimmed(&format!(
                    "Estimated test time: ~{}:{:02} (4 providers, {}s/direction)",
                    mins, secs, per_dir_secs
                ))
            );
        } else {
            println!(
                "  Estimated test time: ~{}:{:02} (4 providers, {}s/direction)",
                mins, secs, per_dir_secs
            );
        }
        println!();
    }

    let state = Arc::new(Mutex::new(DisplayState {
        display: SpeedQXDisplay::new(use_ascii, use_colors, json_mode),
        current_phase: None,
        current_bar: None,
        total_steps,
        use_colors,
        use_ascii,
        json_mode,
        last_provider_num: 0,
    }));

    // Provider completion callback — prints summary after each provider finishes
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

    // Honest exit code (L2): exit non-zero when no provider produced positive
    // throughput in either direction. Mirrors diagnostics/speed.rs's
    // `determine_speed_status` "measured" check — an all-errored / all-zero run
    // is a failure, a slow-but-working link (positive throughput) is success.
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
