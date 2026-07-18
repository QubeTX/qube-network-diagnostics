use clap::{Args, Parser, Subcommand, ValueEnum};

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
        \x20 nd300 dns          Change DNS servers and verify connectivity\n\
        \x20 nd300 -d           Same as 'nd300 dns' (legacy flag form)\n\
        \x20 nd300 fix          Diagnostic-driven triage and recovery loop\n\
        \x20 nd300 -f           Same as 'nd300 fix' (legacy flag form)\n\
        \x20 nd300 update       Check for updates and install\n\
        \x20 nd300 clear-dns    Reset DNS cache\n\
        \x20 nd300 uninstall    Remove nd300 from this system\n\
        \x20 nd300 --fast       Skip speed test for faster execution\n\
        \x20 nd300 --json       Output results as JSON\n\n\
        Run 'nd300 --help' for full details, or 'nd300 -h' for a summary."
)]
pub struct Nd300Cli {
    /// Technician mode - full technical report with deep diagnostics
    #[arg(
        short = 't',
        long = "tech",
        alias = "technician",
        help_heading = "Modes",
        global = true
    )]
    pub tech: bool,

    /// Custom title for the report header
    #[arg(short = 'T', long, help_heading = "Modes", global = true)]
    pub title: Option<String>,

    /// Output results as JSON
    #[arg(long, help_heading = "Output", global = true)]
    pub json: bool,

    /// Use ASCII characters instead of Unicode box-drawing
    #[arg(long, help_heading = "Output", global = true)]
    pub ascii: bool,

    /// Disable colored output
    #[arg(long, help_heading = "Output", global = true)]
    pub no_color: bool,

    /// Show additional debug/trace information
    #[arg(long, help_heading = "Output", global = true)]
    pub verbose: bool,

    /// Skip the speed test (faster execution)
    #[arg(long, help_heading = "Speed Test", global = true)]
    pub fast: bool,

    /// Speed test duration in seconds (per direction, per provider)
    #[arg(long, default_value = "15", help_heading = "Speed Test", global = true)]
    pub speed_duration: u64,

    /// Change DNS servers and verify connectivity
    #[arg(short = 'd', long = "dns", help_heading = "Actions")]
    pub dns: bool,

    /// Diagnostic-driven triage and recovery loop (legacy flag form of `nd300 fix`)
    #[arg(short = 'f', long = "fix", help_heading = "Actions")]
    pub fix: bool,

    /// Clear DNS cache and exit (legacy flag form of `nd300 clear-dns`)
    #[arg(short = 'c', long = "clear-dns", help_heading = "Actions")]
    pub clear_dns: bool,

    /// Uninstall nd300 from this system (legacy flag form of `nd300 uninstall`)
    #[arg(long = "uninstall", help_heading = "Actions")]
    pub uninstall: bool,

    /// Check for updates and install the latest version (legacy flag form of `nd300 update`)
    #[arg(long = "update", help_heading = "Actions")]
    pub update: bool,

    /// Auto-confirm Medium-cost prompts when running the fix flow. Does NOT
    /// bypass High-risk action prompts (Y/N is always required for those).
    #[arg(
        short = 'y',
        long = "yes",
        alias = "non-interactive",
        help_heading = "Actions",
        global = true
    )]
    pub yes: bool,

    /// Print version
    #[arg(short = 'v', long = "version", action = clap::ArgAction::Version)]
    pub version: (),

    /// Action subcommand. If present, takes precedence over the legacy action flags.
    #[command(subcommand)]
    pub command: Option<Nd300Command>,
}

/// Action subcommands. These are equivalent to the long-running action flags
/// (`-d`, `-f`, `--update`, `--clear-dns`, `--uninstall`) — both forms are
/// supported and mean the same thing. The subcommand form is the preferred way
/// going forward; the flag form is kept so existing scripts continue to work.
#[derive(Subcommand, Debug, Clone)]
pub enum Nd300Command {
    /// Diagnostic-driven triage loop: tests → identifies failures → applies
    /// targeted fixes → re-tests → repeats. Bounded by iteration count, wall
    /// clock, and per-action attempt caps. Works for technical and non-technical
    /// users; high-risk actions always require Y/N confirmation.
    Fix(FixArgs),

    /// Change DNS servers and verify connectivity. On success, falls through to
    /// running standard diagnostics (identical to the legacy `-d`/`--dns` flag).
    Dns,

    /// Check for updates and install the latest release.
    Update,

    /// Clear the DNS cache and exit.
    #[command(name = "clear-dns")]
    ClearDns,

    /// Uninstall nd300 from this system.
    Uninstall,

    /// Cross-method install cleanup (HIDDEN). Invoked by the Windows installers
    /// (and the silent self-update path) to consolidate to a single install:
    /// remove a shadowing older `cargo install` copy and/or the other Windows
    /// edition. Safe to run anywhere — it never deletes the running install,
    /// cargo/rustup, the `.cargo\bin` PATH entry, or anything outside the
    /// nd300/speedqx allowlist, and it always exits 0 (cleanup is advisory).
    #[command(name = "migrate-cleanup", hide = true)]
    MigrateCleanup(MigrateArgs),

    /// Remove registered Windows installer channels that conflict with a
    /// deliberate fresh install. Hidden installer-only contract.
    #[command(name = "install-takeover", hide = true)]
    InstallTakeover(InstallTakeoverArgs),

    /// Transaction-bound installer bookkeeping. Hidden installer-only contract.
    #[command(name = "install-maintenance", hide = true)]
    InstallMaintenance(InstallMaintenanceArgs),
}

/// Per-subcommand arguments for `fix`. Currently a placeholder — the `--yes`
/// flag lives at the top level (global), so `nd300 fix --yes`,
/// `nd300 --yes fix`, `nd300 -f -y`, and `nd300 -y -f` all work and share the
/// same field on `Nd300Cli`. New fix-specific options can be added here later
/// without affecting the legacy `-f` flag form.
#[derive(Args, Debug, Clone, Default)]
pub struct FixArgs {}

/// Arguments for the hidden `migrate-cleanup` subcommand. With NO target flag,
/// the command defaults to `--cargo-copy` only (the safest, never-needs-admin
/// consolidation). All flags are tool-agnostic so this contract can be mirrored
/// to TR-300 unchanged.
#[derive(Args, Debug, Clone, Default)]
pub struct MigrateArgs {
    /// Remove a shadowing older `cargo install` / cargo-dist copy in `.cargo\bin`.
    #[arg(long = "cargo-copy")]
    pub cargo_copy: bool,

    /// Remove the OTHER Windows edition (Global perMachine <-> Corporate perUser).
    #[arg(long = "other-edition")]
    pub other_edition: bool,

    /// Suppress human output (installer invokes this so the wizard stays clean).
    /// Ignored when `--json` is set.
    #[arg(long = "quiet")]
    pub quiet: bool,

    /// Detect and report what WOULD be removed without deleting anything.
    #[arg(long = "dry-run")]
    pub dry_run: bool,

    /// Emit a machine-readable JSON report instead of human text.
    #[arg(long = "json")]
    pub json: bool,

    /// The invoking user's profile dir (e.g. `C:\Users\alice`). Used to resolve
    /// that user's `.cargo\bin` and `%LocalAppData%` when this process runs as a
    /// different user (a perMachine installer launched elevated / as SYSTEM).
    #[arg(long = "user-profile", value_name = "PATH")]
    pub user_profile: Option<String>,

    /// The invoking user's CARGO_HOME (e.g. `C:\Users\alice\.cargo`). Takes
    /// precedence over `--user-profile` for locating the cargo-bin directory.
    #[arg(long = "cargo-home", value_name = "PATH")]
    pub cargo_home: Option<String>,

    /// Installer-declared origin for classifying custom install locations.
    /// This is an internal installer contract, not a user-facing option.
    #[arg(long = "install-origin", value_enum, hide = true)]
    pub install_origin: Option<MigrateInstallOrigin>,

    /// Remove versioned images retired while a running Windows binary was
    /// upgraded in place. This is an internal installer contract.
    #[arg(long = "retired-update", hide = true)]
    pub retired_update: bool,
}

/// Valid installer origins accepted by the hidden migration helper.
///
/// Keep the Windows values in lockstep with the four Windows installers and
/// `actions::update::InstallOrigin::json_id`; `macos-pkg` is reserved for the
/// signed Apple Installer postinstall migration hook.
#[derive(ValueEnum, Debug, Clone, Copy, PartialEq, Eq)]
pub enum MigrateInstallOrigin {
    MsiGlobal,
    MsiCorporate,
    ExeGlobal,
    ExeCorporate,
    MacosPkg,
}

/// Arguments for the hidden pre-install channel-takeover helper.
#[derive(Args, Debug, Clone)]
pub struct InstallTakeoverArgs {
    /// Channel that the fresh installer intends to make authoritative.
    #[arg(long, value_enum, hide = true)]
    pub target: InstallTakeoverTarget,

    /// Registry scope visible to this installer phase.
    #[arg(long, value_enum, default_value = "all", hide = true)]
    pub scope: InstallTakeoverScope,
}

/// Fresh-install channels understood by the Windows takeover helper.
#[derive(ValueEnum, Debug, Clone, Copy, PartialEq, Eq)]
pub enum InstallTakeoverTarget {
    MsiGlobal,
    MsiCorporate,
    ExeGlobal,
    ExeCorporate,
    Standalone,
}

/// Registry scope processed by one takeover-helper invocation.
#[derive(ValueEnum, Debug, Clone, Copy, PartialEq, Eq)]
pub enum InstallTakeoverScope {
    All,
    User,
    Machine,
}

/// Arguments for hidden post-install/uninstall bookkeeping.
#[derive(Args, Debug, Clone)]
pub struct InstallMaintenanceArgs {
    #[arg(long, value_enum, hide = true)]
    pub action: InstallMaintenanceAction,

    #[arg(long, value_enum, hide = true)]
    pub origin: MigrateInstallOrigin,

    #[arg(long, value_name = "PATH", hide = true)]
    pub current_path: Option<String>,

    #[arg(long, value_name = "PATH", hide = true)]
    pub legacy_path: Option<String>,
}

#[derive(ValueEnum, Debug, Clone, Copy, PartialEq, Eq)]
pub enum InstallMaintenanceAction {
    SetMarker,
    ClearMarker,
    RepairGlobalPath,
}

/// SpeedQX Internet Speed Test - QubeTX Developer Tools
///
/// Eight-provider speed test (SpeedQX Methodology v4) using Cloudflare, M-Lab
/// NDT7, M-Lab MSAK, LibreSpeed, fast.com, CacheFly, Vultr, and Apple
/// networkQuality.
#[derive(Parser)]
#[command(
    name = "speedqx",
    author,
    version,
    disable_version_flag = true,
    about = "SpeedQX Internet Speed Test - QubeTX Developer Tools",
    long_about = "SpeedQX Internet Speed Test - QubeTX Developer Tools\n\n\
        Eight-provider speed test (SpeedQX Methodology v4) using Cloudflare, M-Lab NDT7,\n\
        M-Lab MSAK (multi-stream), LibreSpeed, fast.com (Netflix), CacheFly, Vultr, and\n\
        Apple networkQuality. Providers run sequentially and are merged (capacity +\n\
        consensus with confidence intervals) for maximum accuracy. --fast runs a quick\n\
        three-provider subset with early stopping.",
    after_long_help = "EXAMPLES:\n\
        \x20 speedqx                     Run the full 8-provider test\n\
        \x20 speedqx --fast              Quick run (Cloudflare + NDT7 + MSAK, early-stop)\n\
        \x20 speedqx --duration 60       60s per direction for the fixed-duration providers\n\
        \x20 speedqx --fastcom-duration 30  Override fast.com to 30s/dir\n\
        \x20 speedqx --skip-msak --skip-apple  Drop MSAK + Apple from the full run\n\
        \x20 speedqx update              Check for updates and install\n\
        \x20 speedqx --update            Same as 'speedqx update' (legacy flag form)\n\
        \x20 speedqx --json              Output results as JSON\n\n\
        Run 'speedqx --help' for full details, or 'speedqx -h' for a summary."
)]
pub struct SpeedQXCli {
    /// Output results as JSON
    #[arg(long, help_heading = "Output", global = true)]
    pub json: bool,

    /// Use ASCII characters instead of Unicode box-drawing
    #[arg(long, help_heading = "Output", global = true)]
    pub ascii: bool,

    /// Disable colored output
    #[arg(long, help_heading = "Output", global = true)]
    pub no_color: bool,

    /// Test duration per direction for CF/NDT7/LibreSpeed: seconds or "auto"
    #[arg(
        long,
        default_value = "30",
        value_parser = parse_duration,
        help_heading = "Speed Test"
    )]
    pub duration: TestDuration,

    /// Test duration per direction for fast.com: seconds or "auto" (default: auto)
    #[arg(
        long,
        default_value = "auto",
        value_parser = parse_duration,
        help_heading = "Speed Test"
    )]
    pub fastcom_duration: TestDuration,

    /// Number of latency probes (advisory; the dense engine is duration-scaled)
    #[arg(long, default_value = "20", help_heading = "Speed Test")]
    pub latency_probes: u32,

    /// FAST mode: Cloudflare + NDT7 + MSAK with early-stop (default is the full
    /// 8-provider run)
    #[arg(long = "fast", help_heading = "Speed Test")]
    pub fast: bool,

    /// Skip the M-Lab MSAK multi-stream provider
    #[arg(long = "skip-msak", help_heading = "Speed Test")]
    pub skip_msak: bool,

    /// Skip the Apple networkQuality provider
    #[arg(long = "skip-apple", help_heading = "Speed Test")]
    pub skip_apple: bool,

    /// Check for updates and install the latest version (legacy flag form of `speedqx update`)
    #[arg(long = "update", help_heading = "Actions")]
    pub update: bool,

    /// Print version
    #[arg(short = 'v', long = "version", action = clap::ArgAction::Version)]
    pub version: (),

    /// Action subcommand. If present, takes precedence over the legacy `--update` flag.
    #[command(subcommand)]
    pub command: Option<SpeedQXCommand>,
}

/// Action subcommands for `speedqx`. Mirrors the `nd300` pattern: subcommand
/// form is preferred, legacy `--update` flag is kept so existing scripts work.
#[derive(Subcommand, Debug, Clone)]
pub enum SpeedQXCommand {
    /// Check for updates and install the latest release.
    Update,
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
