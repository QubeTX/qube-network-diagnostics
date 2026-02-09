use clap::Parser;
use nd_300::config::{Config, OutputFormat};
use nd_300::diagnostics::{self, DiagnosticResults, DiagnosticStatus};
use nd_300::render;

/// ND-300: Cross-platform network diagnostic tool
#[derive(Parser)]
#[command(name = "nd300")]
#[command(author, version, about = "ND-300 Network Diagnostic - QubeTX Developer Tools")]
struct Cli {
    /// Technician mode - show full technical report with deep diagnostics
    #[arg(short = 't', long = "tech", alias = "technician")]
    tech: bool,

    /// Custom title for the report header
    #[arg(short = 'T', long)]
    title: Option<String>,

    /// Output results as JSON
    #[arg(long)]
    json: bool,

    /// Use ASCII characters instead of Unicode box-drawing
    #[arg(long)]
    ascii: bool,

    /// Disable colored output
    #[arg(long)]
    no_color: bool,

    /// Skip the speed test (faster execution)
    #[arg(long)]
    fast: bool,

    /// Speed test duration in seconds
    #[arg(long, default_value = "10")]
    speed_duration: u64,

    /// Show additional debug/trace information
    #[arg(long)]
    verbose: bool,

    /// Clear DNS cache and exit
    #[arg(short = 'c', long = "clear-dns")]
    clear_dns: bool,

    /// Multi-stage network fix: graduated recovery from service restart to stack reset
    #[arg(short = 'f', long = "fix")]
    fix: bool,

    /// Uninstall nd300 from this system
    #[arg(long = "uninstall")]
    uninstall: bool,
}

#[tokio::main]
async fn main() {
    let cli = Cli::parse();

    #[cfg(windows)]
    enable_utf8_console();

    let mut config = Config::new().with_colors(!cli.no_color);

    if cli.ascii {
        config = config.with_ascii();
    }
    if cli.json {
        config = config.with_json();
    }
    if cli.tech {
        config = config.with_tech_mode();
    }
    if cli.fast {
        config = config.with_skip_speed();
    }
    if cli.verbose {
        config = config.with_verbose();
    }
    if let Some(title) = cli.title {
        config = config.with_title(title);
    }
    config = config.with_speed_duration(cli.speed_duration);

    // Action flags: exit early without running diagnostics
    if cli.uninstall {
        let exit_code = nd_300::actions::uninstall::run(&config).await;
        std::process::exit(exit_code);
    }
    if cli.fix {
        let exit_code = nd_300::actions::fix::run(&config).await;
        std::process::exit(exit_code);
    }
    if cli.clear_dns {
        let exit_code = nd_300::actions::clear_dns::run(&config).await;
        std::process::exit(exit_code);
    }

    let results = diagnostics::run_all(&config).await;

    let output = match config.format {
        OutputFormat::Table => {
            if config.is_tech_mode() {
                render::tech_mode::render(&results, &config)
            } else {
                render::user_mode::render(&results, &config)
            }
        }
        OutputFormat::Json => render::json::render(&results, &config),
    };

    print!("{}", output);

    let exit_code = determine_exit_code(&results);
    std::process::exit(exit_code);
}

fn determine_exit_code(results: &DiagnosticResults) -> i32 {
    let statuses = [
        &results.adapters.status,
        &results.interfaces.status,
        &results.gateway.status,
        &results.dns.status,
        &results.public_ip.status,
        &results.latency.status,
        &results.speed.status,
        &results.ports.status,
    ];

    if statuses.iter().any(|s| **s == DiagnosticStatus::Fail) {
        2
    } else if statuses.iter().any(|s| **s == DiagnosticStatus::Warn) {
        1
    } else {
        0
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
