use clap::Parser;

use crate::speedtest::TestDuration;

/// ND-300: Cross-platform network diagnostic tool
#[derive(Parser)]
#[command(
    name = "nd300",
    author,
    version,
    disable_version_flag = true,
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
pub struct Nd300Cli {
    /// Technician mode - full technical report with deep diagnostics
    #[arg(short = 't', long = "tech", alias = "technician", help_heading = "Modes")]
    pub tech: bool,

    /// Custom title for the report header
    #[arg(short = 'T', long, help_heading = "Modes")]
    pub title: Option<String>,

    /// Output results as JSON
    #[arg(long, help_heading = "Output")]
    pub json: bool,

    /// Use ASCII characters instead of Unicode box-drawing
    #[arg(long, help_heading = "Output")]
    pub ascii: bool,

    /// Disable colored output
    #[arg(long, help_heading = "Output")]
    pub no_color: bool,

    /// Show additional debug/trace information
    #[arg(long, help_heading = "Output")]
    pub verbose: bool,

    /// Skip the speed test (faster execution)
    #[arg(long, help_heading = "Speed Test")]
    pub fast: bool,

    /// Speed test duration in seconds
    #[arg(long, default_value = "10", help_heading = "Speed Test")]
    pub speed_duration: u64,

    /// Change DNS servers and verify connectivity
    #[arg(short = 'd', long = "dns", help_heading = "Actions")]
    pub dns: bool,

    /// Multi-stage network fix: graduated recovery from service restart to stack reset
    #[arg(short = 'f', long = "fix", help_heading = "Actions")]
    pub fix: bool,

    /// Clear DNS cache and exit
    #[arg(short = 'c', long = "clear-dns", help_heading = "Actions")]
    pub clear_dns: bool,

    /// Uninstall nd300 from this system
    #[arg(long = "uninstall", help_heading = "Actions")]
    pub uninstall: bool,

    /// Print version
    #[arg(short = 'v', long = "version", action = clap::ArgAction::Version)]
    pub version: (),
}

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
        Dual-provider speed test using Cloudflare and M-Lab NDT7.",
    after_long_help = "EXAMPLES:\n\
        \x20 speedqx                Run full speed test (Cloudflare + NDT7)\n\
        \x20 speedqx --cf-only      Cloudflare only\n\
        \x20 speedqx --ndt-only     M-Lab NDT7 only\n\
        \x20 speedqx --duration 60  60-second test per provider\n\
        \x20 speedqx --json         Output results as JSON\n\n\
        Run 'speedqx --help' for full details, or 'speedqx -h' for a summary."
)]
pub struct SpeedQXCli {
    /// Output results as JSON
    #[arg(long, help_heading = "Output")]
    pub json: bool,

    /// Use ASCII characters instead of Unicode box-drawing
    #[arg(long, help_heading = "Output")]
    pub ascii: bool,

    /// Disable colored output
    #[arg(long, help_heading = "Output")]
    pub no_color: bool,

    /// Test duration per provider: seconds or "auto"
    #[arg(
        long,
        default_value = "30",
        value_parser = parse_duration,
        help_heading = "Speed Test"
    )]
    pub duration: TestDuration,

    /// Use only Cloudflare (skip NDT7)
    #[arg(long, conflicts_with = "ndt_only", help_heading = "Speed Test")]
    pub cf_only: bool,

    /// Use only M-Lab NDT7 (skip Cloudflare)
    #[arg(long, conflicts_with = "cf_only", help_heading = "Speed Test")]
    pub ndt_only: bool,

    /// Number of latency probes
    #[arg(long, default_value = "20", help_heading = "Speed Test")]
    pub latency_probes: u32,

    /// Print version
    #[arg(short = 'v', long = "version", action = clap::ArgAction::Version)]
    pub version: (),
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
