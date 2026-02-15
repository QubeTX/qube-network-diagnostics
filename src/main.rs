use clap::Parser;
use nd_300::config::{Config, OutputFormat};
use nd_300::diagnostics::{self, DiagnosticResults, DiagnosticStatus};
use nd_300::render;

/// ND-300: Cross-platform network diagnostic tool
#[derive(Parser)]
#[command(
    name = "nd300",
    author,
    version,
    about = "ND-300 Network Diagnostic - QubeTX Developer Tools",
    long_about = "ND-300 Network Diagnostic - QubeTX Developer Tools\n\n\
        Cross-platform network diagnostics with 25+ concurrent checks,\n\
        multi-stage network repair, and DNS configuration management.",
    after_long_help = "EXAMPLES:\n\
        \x20 nd300              Run standard diagnostics\n\
        \x20 nd300 -t           Technician mode (deep diagnostics)\n\
        \x20 nd300 -d           Change DNS configuration\n\
        \x20 nd300 -f           Multi-stage network fix\n\
        \x20 nd300 --fast       Skip speed test for faster execution\n\
        \x20 nd300 --json       Output results as JSON\n\n\
        Run 'nd300 --help' for full details, or 'nd300 -h' for a summary."
)]
struct Cli {
    /// Technician mode - full technical report with deep diagnostics
    #[arg(short = 't', long = "tech", alias = "technician", help_heading = "Modes")]
    tech: bool,

    /// Custom title for the report header
    #[arg(short = 'T', long, help_heading = "Modes")]
    title: Option<String>,

    /// Output results as JSON
    #[arg(long, help_heading = "Output")]
    json: bool,

    /// Use ASCII characters instead of Unicode box-drawing
    #[arg(long, help_heading = "Output")]
    ascii: bool,

    /// Disable colored output
    #[arg(long, help_heading = "Output")]
    no_color: bool,

    /// Show additional debug/trace information
    #[arg(long, help_heading = "Output")]
    verbose: bool,

    /// Skip the speed test (faster execution)
    #[arg(long, help_heading = "Speed Test")]
    fast: bool,

    /// Speed test duration in seconds
    #[arg(long, default_value = "10", help_heading = "Speed Test")]
    speed_duration: u64,

    /// Change DNS servers and verify connectivity
    #[arg(short = 'd', long = "dns", help_heading = "Actions")]
    dns: bool,

    /// Multi-stage network fix: graduated recovery from service restart to stack reset
    #[arg(short = 'f', long = "fix", help_heading = "Actions")]
    fix: bool,

    /// Clear DNS cache and exit
    #[arg(short = 'c', long = "clear-dns", help_heading = "Actions")]
    clear_dns: bool,

    /// Uninstall nd300 from this system
    #[arg(long = "uninstall", help_heading = "Actions")]
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

    // Semi-exit-early: exits on failure, falls through to diagnostics on success
    if cli.dns {
        let dns_result = nd_300::actions::dns::run(&config).await;
        if dns_result != 0 {
            std::process::exit(dns_result);
        }
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
