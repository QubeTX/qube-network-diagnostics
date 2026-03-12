use clap::Parser;
use nd_300::speedtest::display::{render_results, SpeedQXDisplay};
use nd_300::speedtest::{Phase, SpeedTestConfig, TestDuration};
use std::sync::Mutex;

/// SpeedQX Internet Speed Test - QubeTX Developer Tools
///
/// Dual-provider speed test using Cloudflare and M-Lab NDT7.
#[derive(Parser)]
#[command(
    name = "speedqx",
    author,
    version,
    disable_version_flag = true,
    about = "SpeedQX Internet Speed Test - QubeTX Developer Tools",
    long_about = "SpeedQX Internet Speed Test - QubeTX Developer Tools\n\n\
        Dual-provider speed test using Cloudflare and M-Lab NDT7."
)]
struct Cli {
    /// Output results as JSON
    #[arg(long, help_heading = "Output")]
    json: bool,

    /// Use ASCII characters instead of Unicode box-drawing
    #[arg(long, help_heading = "Output")]
    ascii: bool,

    /// Disable colored output
    #[arg(long, help_heading = "Output")]
    no_color: bool,

    /// Test duration per provider: seconds or "auto"
    #[arg(
        long,
        default_value = "30",
        value_parser = parse_duration,
        help_heading = "Speed Test"
    )]
    duration: TestDuration,

    /// Use only Cloudflare (skip NDT7)
    #[arg(long, conflicts_with = "ndt_only", help_heading = "Speed Test")]
    cf_only: bool,

    /// Use only M-Lab NDT7 (skip Cloudflare)
    #[arg(long, conflicts_with = "cf_only", help_heading = "Speed Test")]
    ndt_only: bool,

    /// Number of latency probes
    #[arg(long, default_value = "20", help_heading = "Speed Test")]
    latency_probes: u32,

    /// Print version
    #[arg(short = 'v', long = "version", action = clap::ArgAction::Version)]
    version: (),
}

fn parse_duration(s: &str) -> Result<TestDuration, String> {
    if s.eq_ignore_ascii_case("auto") {
        Ok(TestDuration::Auto)
    } else {
        s.parse::<u64>()
            .map(TestDuration::Seconds)
            .map_err(|_| format!("invalid duration '{}': expected a number or \"auto\"", s))
    }
}

/// Tracks which phase is currently active so the callback can manage transitions.
struct DisplayState {
    display: SpeedQXDisplay,
    current_phase: Option<Phase>,
    current_bar: Option<indicatif::ProgressBar>,
    total_steps: u32,
    cf_only: bool,
    ndt_only: bool,
}

impl DisplayState {
    fn step_for_phase(&self, phase: Phase) -> u32 {
        if self.cf_only {
            match phase {
                Phase::CfLatency => 1,
                Phase::CfDownload => 2,
                Phase::CfUpload => 3,
                Phase::Computing => 4,
                _ => 0,
            }
        } else if self.ndt_only {
            match phase {
                Phase::Ndt7Discovery => 1,
                Phase::Ndt7Download => 2,
                Phase::Ndt7Upload => 3,
                Phase::Computing => 4,
                _ => 0,
            }
        } else {
            match phase {
                Phase::CfLatency => 1,
                Phase::CfDownload => 2,
                Phase::CfUpload => 3,
                Phase::Ndt7Discovery => 4,
                Phase::Ndt7Download => 5,
                Phase::Ndt7Upload => 6,
                Phase::Computing => 7,
            }
        }
    }

    fn label_for_phase(&self, phase: Phase) -> &'static str {
        match phase {
            Phase::CfLatency => "Latency",
            Phase::CfDownload => "Download (Cloudflare)",
            Phase::CfUpload => "Upload (Cloudflare)",
            Phase::Ndt7Discovery => "Server discovery",
            Phase::Ndt7Download => "Download (M-Lab NDT7)",
            Phase::Ndt7Upload => "Upload (M-Lab NDT7)",
            Phase::Computing => "Results computed",
        }
    }

    fn is_progress_phase(&self, phase: Phase) -> bool {
        matches!(
            phase,
            Phase::CfDownload | Phase::CfUpload | Phase::Ndt7Download | Phase::Ndt7Upload
        )
    }

    fn handle_phase(&mut self, phase: Phase, progress: f64) {
        let step = self.step_for_phase(phase);
        if step == 0 {
            return;
        }

        // Phase transition: finish previous bar and print completion line
        if self.current_phase != Some(phase) {
            self.finish_current();
            self.current_phase = Some(phase);

            let label = self.label_for_phase(phase);

            if self.is_progress_phase(phase) {
                let bar = self.display.create_progress_bar(step, self.total_steps, label);
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
    let cli = Cli::parse();

    #[cfg(windows)]
    enable_utf8_console();

    let use_ascii = cli.ascii;
    let use_colors = !cli.no_color;
    let json_mode = cli.json;

    let config = SpeedTestConfig {
        duration: cli.duration,
        latency_probes: cli.latency_probes,
        run_cloudflare: !cli.ndt_only,
        run_ndt7: !cli.cf_only,
        use_colors,
    };

    let total_steps = if cli.cf_only || cli.ndt_only { 4 } else { 7 };

    let display = SpeedQXDisplay::new(use_ascii, use_colors, json_mode);
    display.print_header();

    let state = Mutex::new(DisplayState {
        display: SpeedQXDisplay::new(use_ascii, use_colors, json_mode),
        current_phase: None,
        current_bar: None,
        total_steps,
        cf_only: cli.cf_only,
        ndt_only: cli.ndt_only,
    });

    let result = nd_300::speedtest::run(config, move |phase, progress| {
        if let Ok(mut s) = state.lock() {
            s.handle_phase(phase, progress);
        }
    })
    .await;

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
