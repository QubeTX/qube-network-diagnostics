// Self-update for ND-300 (nd300 + speedqx).
//
// Checks the GitHub releases API for a newer version and dispatches to an
// installer strategy that matches how this binary was installed. On Windows
// (v3.1.0+), the four first-class installers (MSI Global, MSI Corporate,
// EXE Global, EXE Corporate) write a HKCU\Software\ND300\InstallSource
// registry marker, which `detect_install_origin()` reads to pick the
// matching MSI/EXE for in-place upgrade. Path-based fallback handles the
// cargo install / PowerShell installer path that doesn't write a marker.
//
// This module is shared by both the `nd300` and `speedqx` binaries
// (`cargo install nd300` and every installer ship both), so all of this
// benefits both. ND-300's richer cargo shadow-cleanup machinery
// (`ShadowCleanupDecision`, `classify_shadow_cleanup`, …) is preserved on
// top of the TR-300-ported MSI/EXE + SHA-256-verify + prerelease-semver
// capabilities.
//
// LOCKSTEP CONTRACT: if the install paths or registry-marker values change
// in wix/main.wxs, wix-corporate/corporate.wxs, inno/global.iss, or
// inno/corporate.iss, update `detect_install_origin()` /
// `classify_install_path()` / `read_install_source_marker()` here to match.

use crate::config::{Config, OutputFormat};
use crate::render::color;
use crate::VERSION;

use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

use super::{fail_icon, success_icon};

/// GitHub API endpoint for the latest release.
const RELEASES_URL: &str =
    "https://api.github.com/repos/QubeTX/qube-network-diagnostics/releases/latest";

/// Shell installer URL (macOS/Linux). The `releases/latest` redirect always
/// resolves to the newest published release, so this URL is permanently stable.
#[cfg(not(windows))]
const SHELL_INSTALLER: &str =
    "https://github.com/QubeTX/qube-network-diagnostics/releases/latest/download/nd300-installer.sh";

/// PowerShell installer URL (Windows). Same `releases/latest` redirect.
#[cfg(windows)]
const PS_INSTALLER: &str =
    "https://github.com/QubeTX/qube-network-diagnostics/releases/latest/download/nd300-installer.ps1";

/// Global (perMachine) MSI installer URL.
#[cfg(windows)]
const MSI_GLOBAL_URL: &str = "https://github.com/QubeTX/qube-network-diagnostics/releases/latest/download/nd300-x86_64-pc-windows-msvc.msi";

/// Corporate (perUser) MSI installer URL.
#[cfg(windows)]
const MSI_CORPORATE_URL: &str = "https://github.com/QubeTX/qube-network-diagnostics/releases/latest/download/nd300-x86_64-pc-windows-msvc-corporate.msi";

/// Global (perMachine) EXE installer URL (Inno Setup).
#[cfg(windows)]
const EXE_GLOBAL_URL: &str = "https://github.com/QubeTX/qube-network-diagnostics/releases/latest/download/nd300-x86_64-pc-windows-msvc-setup.exe";

/// Corporate (perUser) EXE installer URL (Inno Setup).
#[cfg(windows)]
const EXE_CORPORATE_URL: &str = "https://github.com/QubeTX/qube-network-diagnostics/releases/latest/download/nd300-x86_64-pc-windows-msvc-corporate-setup.exe";

const MANUAL_INSTALL_URL: &str = "https://github.com/QubeTX/qube-network-diagnostics#install";

// ─── Strategy types ──────────────────────────────────────────────────────────

/// Ordered candidate strategies for updating the binary.
///
/// For cargo install / shell installer users, the runner tries each in order
/// and falls through on any failure (preflight or runtime) until one succeeds.
/// Cargo-first when available, cargo-dist installer as universal fallback.
///
/// For first-class Windows installs (v3.1.0+), `build_strategy_list` returns a
/// *single* matching MSI/EXE strategy (no cross-fall-back between installer
/// types — re-running a different product would create coexistence problems:
/// two ARP entries, PATH ordering decides which wins).
// The four MSI/EXE variants are only ever constructed inside the #[cfg(windows)]
// block of build_strategy_list(), so on non-Windows targets the dead_code lint
// flags them as never-constructed. The variants still need to exist on every
// platform so the label()/json_id()/json_method() match arms stay exhaustive and
// the try_strategy() dispatch arms compile. cfg_attr keeps Windows clippy strict
// (missing wiring on Windows still trips the lint) while silencing it elsewhere.
#[cfg_attr(not(windows), allow(dead_code))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum UpdateStrategy {
    Cargo,
    InstallerCurl,
    InstallerWget,
    InstallerPowerShell,
    InstallerPwsh,
    /// Re-runs the Global perMachine MSI (UAC required).
    MsiGlobal,
    /// Re-runs the Corporate perUser MSI (no UAC required).
    MsiCorporate,
    /// Re-runs the Global perMachine Inno Setup EXE (UAC required).
    ExeGlobal,
    /// Re-runs the Corporate perUser Inno Setup EXE (no UAC required).
    ExeCorporate,
}

impl UpdateStrategy {
    fn label(self) -> &'static str {
        match self {
            UpdateStrategy::Cargo => "cargo install",
            UpdateStrategy::InstallerCurl => "curl shell installer",
            UpdateStrategy::InstallerWget => "wget shell installer",
            UpdateStrategy::InstallerPowerShell => "PowerShell installer",
            UpdateStrategy::InstallerPwsh => "pwsh installer",
            UpdateStrategy::MsiGlobal => "Global MSI installer",
            UpdateStrategy::MsiCorporate => "Corporate MSI installer",
            UpdateStrategy::ExeGlobal => "Global EXE installer",
            UpdateStrategy::ExeCorporate => "Corporate EXE installer",
        }
    }

    fn json_id(self) -> &'static str {
        match self {
            UpdateStrategy::Cargo => "cargo",
            UpdateStrategy::InstallerCurl => "installer_curl",
            UpdateStrategy::InstallerWget => "installer_wget",
            UpdateStrategy::InstallerPowerShell => "installer_powershell",
            UpdateStrategy::InstallerPwsh => "installer_pwsh",
            UpdateStrategy::MsiGlobal => "msi_global",
            UpdateStrategy::MsiCorporate => "msi_corporate",
            UpdateStrategy::ExeGlobal => "exe_global",
            UpdateStrategy::ExeCorporate => "exe_corporate",
        }
    }

    /// Backward-compat label for the JSON `"method"` field on success.
    ///
    /// All MSI/EXE strategies map to `"installer"` to match the legacy taxonomy
    /// (anything that isn't `cargo install` was historically called an
    /// "installer"). External consumers wanting the specific installer type
    /// should read the precise `"strategy"` field instead.
    fn json_method(self) -> &'static str {
        if matches!(self, UpdateStrategy::Cargo) {
            "cargo"
        } else {
            "installer"
        }
    }
}

/// Where this binary was installed from, on Windows.
///
/// Determines which installer `nd300 update` downloads and re-runs for an
/// in-place upgrade. First-class installers (the four MSI/EXE variants) write a
/// `HKCU\Software\ND300\InstallSource` registry marker on install;
/// `detect_install_origin()` reads that marker. A path-based fallback covers the
/// cargo install / PowerShell installer path that doesn't write a marker.
#[cfg(windows)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum InstallOrigin {
    /// `C:\Program Files\nd300\bin\nd300.exe`, installed from wix/main.wxs.
    MsiGlobal,
    /// `%LocalAppData%\Programs\nd300\bin\nd300.exe`, from wix-corporate/corporate.wxs.
    MsiCorporate,
    /// `C:\Program Files\nd300\bin\nd300.exe`, from inno/global.iss.
    ExeGlobal,
    /// `%LocalAppData%\Programs\nd300\bin\nd300.exe`, from inno/corporate.iss.
    ExeCorporate,
    /// `~\.cargo\bin\nd300.exe` — installed via `cargo install` or the
    /// cargo-dist PowerShell installer. Uses the legacy strategy chain.
    CargoOrInstaller,
    /// Couldn't determine origin (custom install location, portable use, etc.).
    /// Treated like `CargoOrInstaller` so the legacy chain runs.
    Unknown,
}

#[cfg(windows)]
impl InstallOrigin {
    /// String form for JSON output. Matches the registry marker values written
    /// by the installers; `cargo-or-installer` / `unknown` are synthesized by the
    /// path-based fallback.
    fn json_id(self) -> &'static str {
        match self {
            InstallOrigin::MsiGlobal => "msi-global",
            InstallOrigin::MsiCorporate => "msi-corporate",
            InstallOrigin::ExeGlobal => "exe-global",
            InstallOrigin::ExeCorporate => "exe-corporate",
            InstallOrigin::CargoOrInstaller => "cargo-or-installer",
            InstallOrigin::Unknown => "unknown",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TargetOs {
    Unix,
    Windows,
}

#[derive(Debug)]
enum StrategyError {
    /// Launcher / required tool not found on PATH or capability check failed.
    /// Safe to fall through — nothing was written.
    Preflight(String),
    /// The strategy ran end-to-end but reported a non-zero exit (or a verify
    /// mismatch). cargo and cargo-dist installers are both atomic (write-to-temp,
    /// move-into-place), so it's still safe to fall through to the next strategy.
    Runtime(String),
    /// The strategy reached a state where falling through would hide an
    /// incomplete install, such as a successful cargo install shadowed by an
    /// old installer-managed binary that could not be removed.
    Fatal(String),
}

#[derive(Debug)]
enum AttemptKind {
    Skipped,
    Failed,
}

#[derive(Debug)]
struct AttemptRecord {
    strategy: UpdateStrategy,
    kind: AttemptKind,
    message: String,
}

#[derive(Debug)]
struct UpdateFailure {
    attempts: Vec<AttemptRecord>,
}

// ─── Public entry point ──────────────────────────────────────────────────────

pub async fn run(config: &Config) -> i32 {
    if config.format == OutputFormat::Json {
        return run_json(config).await;
    }

    println!();
    println!("  {} Checking for updates...", color::cyan("*", config));

    let latest = match fetch_latest_version().await {
        Ok(v) => v,
        Err(e) => {
            println!(
                "  {} {}",
                color::red(fail_icon(config), config),
                color::red(&format!("Failed to check for updates: {}", e), config),
            );
            return 2;
        }
    };

    let current = VERSION.to_string();

    if !is_newer(&current, &latest) {
        println!(
            "  {} {}",
            color::green(success_icon(config), config),
            color::green(
                &format!("Already on the latest version (v{})", current),
                config
            ),
        );
        return 0;
    }

    println!(
        "  {} Update available: v{} {} v{}",
        color::cyan("*", config),
        current,
        color::cyan("->", config),
        latest,
    );

    let strategies = build_strategy_list();
    let primary = strategies.first().copied();
    if let Some(s) = primary {
        println!(
            "  {} Updating via {}...",
            color::cyan("*", config),
            s.label()
        );
    }
    println!();

    match execute_update(&latest, &strategies) {
        Ok(used) => {
            println!();
            println!(
                "  {} {}",
                color::green(success_icon(config), config),
                color::green(
                    &format!("Updated to v{} via {}", latest, used.label()),
                    config
                ),
            );
            0
        }
        Err(failure) => {
            println!();
            println!(
                "  {} {}",
                color::red(fail_icon(config), config),
                color::red("Update failed. Strategies attempted:", config),
            );
            for record in &failure.attempts {
                let kind = match record.kind {
                    AttemptKind::Skipped => "skipped",
                    AttemptKind::Failed => "failed",
                };
                println!(
                    "      · {} — {}: {}",
                    record.strategy.label(),
                    kind,
                    record.message
                );
            }
            println!();
            println!("  To update manually, see: {}", MANUAL_INSTALL_URL);
            2
        }
    }
}

async fn run_json(_config: &Config) -> i32 {
    let latest = match fetch_latest_version().await {
        Ok(v) => v,
        Err(e) => {
            let mut output = serde_json::json!({
                "action": "update",
                "success": false,
                "message": format!("Failed to check for updates: {}", e),
                "current_version": VERSION,
            });
            inject_install_origin(&mut output);
            println!(
                "{}",
                serde_json::to_string_pretty(&output).unwrap_or_else(|_| "{}".to_string())
            );
            return 2;
        }
    };

    let current = VERSION.to_string();
    let update_available = is_newer(&current, &latest);

    if !update_available {
        let mut output = serde_json::json!({
            "action": "update",
            "success": true,
            "message": "Already on the latest version",
            "current_version": current,
            "latest_version": latest,
            "update_available": false,
        });
        inject_install_origin(&mut output);
        println!(
            "{}",
            serde_json::to_string_pretty(&output).unwrap_or_else(|_| "{}".to_string())
        );
        return 0;
    }

    let strategies = build_strategy_list();
    match execute_update(&latest, &strategies) {
        Ok(used) => {
            let mut output = serde_json::json!({
                "action": "update",
                "success": true,
                "message": format!("Updated from v{} to v{}", current, latest),
                "current_version": current,
                "latest_version": latest,
                "update_available": true,
                "method": used.json_method(),
                "strategy": used.json_id(),
            });
            inject_install_origin(&mut output);
            println!(
                "{}",
                serde_json::to_string_pretty(&output).unwrap_or_else(|_| "{}".to_string())
            );
            0
        }
        Err(failure) => {
            let attempts: Vec<serde_json::Value> = failure
                .attempts
                .iter()
                .map(|r| {
                    serde_json::json!({
                        "strategy": r.strategy.json_id(),
                        "result": match r.kind {
                            AttemptKind::Skipped => "skipped",
                            AttemptKind::Failed => "failed",
                        },
                        "message": r.message,
                    })
                })
                .collect();
            let mut output = serde_json::json!({
                "action": "update",
                "success": false,
                "message": "Update failed; see attempts",
                "current_version": current,
                "latest_version": latest,
                "update_available": true,
                "attempts": attempts,
            });
            inject_install_origin(&mut output);
            println!(
                "{}",
                serde_json::to_string_pretty(&output).unwrap_or_else(|_| "{}".to_string())
            );
            2
        }
    }
}

/// Add a top-level `install_origin` field to the JSON payload on Windows.
/// On other platforms this is a no-op — the field is Windows-only because
/// install-origin only meaningfully varies on Windows (where users have a
/// choice of MSI vs EXE installer, perMachine vs perUser scope).
#[cfg(windows)]
fn inject_install_origin(payload: &mut serde_json::Value) {
    if let Some(obj) = payload.as_object_mut() {
        obj.insert(
            "install_origin".to_string(),
            serde_json::Value::String(detect_install_origin().json_id().to_string()),
        );
    }
}

#[cfg(not(windows))]
fn inject_install_origin(_payload: &mut serde_json::Value) {
    // No-op on non-Windows; the field would always be the same value.
}

// ─── Version probing ─────────────────────────────────────────────────────────

/// Turn an HTTP status + rate-limit header into an actionable message. The most
/// common intermittent failure is GitHub's unauthenticated rate limit (60
/// requests/hour per IP) — worth naming explicitly so a user who "ran update and
/// it didn't work" knows to just wait. Pure + testable.
fn http_status_message(code: u16, ratelimit_remaining: Option<&str>) -> String {
    if code == 403 && ratelimit_remaining == Some("0") {
        "GitHub API rate limit exceeded (unauthenticated requests are capped at 60/hour per IP). Wait for the limit to reset and try again, or update manually.".to_string()
    } else {
        format!(
            "GitHub API returned HTTP {} when checking for updates",
            code
        )
    }
}

async fn fetch_latest_version() -> Result<String, String> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(15))
        .build()
        .map_err(|e| format!("HTTP client error: {}", e))?;

    let resp = client
        .get(RELEASES_URL)
        .header("User-Agent", format!("nd300/{}", VERSION))
        .header("Accept", "application/vnd.github+json")
        .send()
        .await
        .map_err(|e| format!("Request failed: {}", e))?;

    if !resp.status().is_success() {
        // Surface the rate-limit case explicitly (reqwest exposes the response
        // headers + status even on a non-2xx, so we read x-ratelimit-remaining).
        let code = resp.status().as_u16();
        let remaining = resp
            .headers()
            .get("x-ratelimit-remaining")
            .and_then(|v| v.to_str().ok())
            .map(|s| s.to_string());
        return Err(http_status_message(code, remaining.as_deref()));
    }

    let body: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| format!("Failed to parse response: {}", e))?;

    let tag = body["tag_name"]
        .as_str()
        .ok_or("Missing tag_name in response")?;

    // Strip leading 'v' if present.
    Ok(tag.strip_prefix('v').unwrap_or(tag).to_string())
}

/// Strip any prerelease (`-rc.1`) or build-metadata (`+nightly.42`) suffix from a
/// semver string, returning just the `MAJOR.MINOR.PATCH` portion.
///
/// A naive `split('.').filter_map(parse)` silently drops non-numeric segments,
/// so `"3.1.2-rc.1".split('.')` would parse to `[3, 1, 1]` (the `2-rc` segment
/// fails to parse, the trailing `1` survives) — identical to `"3.1.1"`, which
/// would strand a user on a prerelease tag. Truncating at the first non-`[0-9.]`
/// character is the simplest robust fix.
fn strip_prerelease_metadata(v: &str) -> &str {
    match v.find(|c: char| !c.is_ascii_digit() && c != '.') {
        Some(idx) => &v[..idx],
        None => v,
    }
}

/// Compare semver versions. Returns true if `latest` is newer than `current`.
///
/// Handles prerelease tags per the semver spec's general intent: a prerelease of
/// an upcoming version is considered NEWER than the previous stable patch
/// (`3.1.2-rc.1` > `3.1.1`), and a stable release IS newer than any prerelease of
/// the same triple (`3.1.2` > `3.1.2-rc.1`). Two prereleases of the same triple
/// are treated as equal — GitHub's `/releases/latest` filters prereleases out, so
/// that case is theoretical.
fn is_newer(current: &str, latest: &str) -> bool {
    let current_stripped = strip_prerelease_metadata(current);
    let latest_stripped = strip_prerelease_metadata(latest);

    let parse =
        |v: &str| -> Vec<u64> { v.split('.').filter_map(|s| s.parse::<u64>().ok()).collect() };

    let c = parse(current_stripped);
    let l = parse(latest_stripped);

    let len = c.len().max(l.len());
    for i in 0..len {
        let cv = c.get(i).copied().unwrap_or(0);
        let lv = l.get(i).copied().unwrap_or(0);
        if lv > cv {
            return true;
        }
        if lv < cv {
            return false;
        }
    }

    // Numeric parts equal. If the current binary is on a prerelease of the same
    // triple (e.g. `3.1.2-rc.1`) and the latest is the stable release (`3.1.2`),
    // the stable IS newer per semver ordering.
    let current_has_suffix = current.len() != current_stripped.len();
    let latest_has_suffix = latest.len() != latest_stripped.len();
    current_has_suffix && !latest_has_suffix
}

// ─── Strategy ordering (pure, testable) ──────────────────────────────────────

fn order_strategies(cargo_invokable: bool, os: TargetOs) -> Vec<UpdateStrategy> {
    let mut list = Vec::new();
    if cargo_invokable {
        list.push(UpdateStrategy::Cargo);
    }
    match os {
        TargetOs::Unix => {
            list.push(UpdateStrategy::InstallerCurl);
            list.push(UpdateStrategy::InstallerWget);
        }
        TargetOs::Windows => {
            list.push(UpdateStrategy::InstallerPowerShell);
            list.push(UpdateStrategy::InstallerPwsh);
        }
    }
    list
}

fn current_target_os() -> TargetOs {
    if cfg!(windows) {
        TargetOs::Windows
    } else {
        TargetOs::Unix
    }
}

fn cargo_invokable() -> bool {
    tool_exists("cargo")
}

fn tool_exists(tool: &str) -> bool {
    Command::new(tool)
        .arg("--version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

fn build_strategy_list() -> Vec<UpdateStrategy> {
    #[cfg(windows)]
    {
        // For first-class Windows installs, dispatch to a single matching
        // installer strategy. Don't cross-fall-back to a different installer type
        // — running a different product would create coexistence problems (two
        // ARP entries, PATH ordering wins).
        match detect_install_origin() {
            InstallOrigin::MsiGlobal => return vec![UpdateStrategy::MsiGlobal],
            InstallOrigin::MsiCorporate => return vec![UpdateStrategy::MsiCorporate],
            InstallOrigin::ExeGlobal => return vec![UpdateStrategy::ExeGlobal],
            InstallOrigin::ExeCorporate => return vec![UpdateStrategy::ExeCorporate],
            InstallOrigin::CargoOrInstaller | InstallOrigin::Unknown => {
                // Fall through to the legacy cargo/PS chain.
            }
        }
    }
    order_strategies(cargo_invokable(), current_target_os())
}

// ─── Strategy execution ──────────────────────────────────────────────────────

fn execute_update(
    latest: &str,
    strategies: &[UpdateStrategy],
) -> Result<UpdateStrategy, UpdateFailure> {
    let mut attempts = Vec::new();
    for &strategy in strategies {
        match try_strategy(strategy, latest) {
            Ok(()) => return Ok(strategy),
            Err(StrategyError::Preflight(msg)) => {
                eprintln!("  · skipped {}: {}", strategy.label(), msg);
                attempts.push(AttemptRecord {
                    strategy,
                    kind: AttemptKind::Skipped,
                    message: msg,
                });
            }
            Err(StrategyError::Runtime(msg)) => {
                eprintln!("  · {} failed: {}", strategy.label(), msg);
                attempts.push(AttemptRecord {
                    strategy,
                    kind: AttemptKind::Failed,
                    message: msg,
                });
            }
            Err(StrategyError::Fatal(msg)) => {
                eprintln!("  · {} failed: {}", strategy.label(), msg);
                attempts.push(AttemptRecord {
                    strategy,
                    kind: AttemptKind::Failed,
                    message: msg,
                });
                return Err(UpdateFailure { attempts });
            }
        }
    }
    Err(UpdateFailure { attempts })
}

fn try_strategy(strategy: UpdateStrategy, latest: &str) -> Result<(), StrategyError> {
    match strategy {
        UpdateStrategy::Cargo => try_cargo_install(latest),
        UpdateStrategy::InstallerCurl => try_installer_curl(),
        UpdateStrategy::InstallerWget => try_installer_wget(),
        UpdateStrategy::InstallerPowerShell => try_installer_powershell("powershell"),
        UpdateStrategy::InstallerPwsh => try_installer_powershell("pwsh"),
        UpdateStrategy::MsiGlobal => try_msi_install(msi_global_url(), latest),
        UpdateStrategy::MsiCorporate => try_msi_install(msi_corporate_url(), latest),
        UpdateStrategy::ExeGlobal => try_exe_install(exe_global_url(), latest),
        UpdateStrategy::ExeCorporate => try_exe_install(exe_corporate_url(), latest),
    }
}

// URL accessors are #[cfg(windows)] under the hood but exposed uniformly so the
// dispatch arms above don't need their own #[cfg] gates — keeps the match
// exhaustive on all platforms.
#[cfg(windows)]
fn msi_global_url() -> &'static str {
    MSI_GLOBAL_URL
}
#[cfg(windows)]
fn msi_corporate_url() -> &'static str {
    MSI_CORPORATE_URL
}
#[cfg(windows)]
fn exe_global_url() -> &'static str {
    EXE_GLOBAL_URL
}
#[cfg(windows)]
fn exe_corporate_url() -> &'static str {
    EXE_CORPORATE_URL
}
#[cfg(not(windows))]
fn msi_global_url() -> &'static str {
    ""
}
#[cfg(not(windows))]
fn msi_corporate_url() -> &'static str {
    ""
}
#[cfg(not(windows))]
fn exe_global_url() -> &'static str {
    ""
}
#[cfg(not(windows))]
fn exe_corporate_url() -> &'static str {
    ""
}

fn try_cargo_install(latest: &str) -> Result<(), StrategyError> {
    rustup_update_stable_best_effort();
    match run_command_capture("cargo", &["install", "nd300", "--force"]) {
        Ok(()) => {
            // Clean up any shadowing non-cargo install first (ND-300's existing
            // machinery), THEN confirm the running binary actually changed. cargo
            // exit 0 doesn't guarantee that: crates.io may still serve the OLD
            // version (publish lag right after a GitHub release), or a different
            // nd300 may be earlier on PATH. Verify, and fall through to the
            // prebuilt installer on mismatch so we don't loop "update available".
            cleanup_shadowing_current_install_after_cargo_success()?;
            verify_cargo_post_install(latest)
        }
        Err(StrategyError::Runtime(msg))
            if cargo_failure_suggests_existing_binary_collision(&msg) =>
        {
            cleanup_current_install_for_cargo_retry()?;
            run_command_capture("cargo", &["install", "nd300", "--force"])?;
            verify_cargo_post_install(latest)
        }
        other => other,
    }
}

/// The three outcomes of a shadow-cleanup attempt, as a function of the cleanup
/// report. Pure so it can be unit-tested without touching the filesystem.
#[derive(Debug, PartialEq, Eq)]
enum ShadowCleanupDecision {
    /// The old shadowing binary is gone now — clean success.
    Removed,
    /// Windows: the old binary is scheduled for deletion when this process
    /// exits. The new cargo binary in cargo-bin will win in a *new* shell. This
    /// is an honest, non-fatal warning, not a silent success.
    Scheduled,
    /// The old binary could not be removed at all — falling through would hide an
    /// incomplete install, so this is Fatal.
    NotRemoved,
}

/// Map a cleanup report to a shadow-cleanup decision. `binary_removed` wins;
/// otherwise a Windows scheduled-on-exit removal is an honest warning; otherwise
/// it's a hard failure.
fn classify_shadow_cleanup(report: &super::uninstall::CleanupReport) -> ShadowCleanupDecision {
    if report.binary_removed {
        ShadowCleanupDecision::Removed
    } else if report.binary_removal_scheduled {
        ShadowCleanupDecision::Scheduled
    } else {
        ShadowCleanupDecision::NotRemoved
    }
}

fn cleanup_shadowing_current_install_after_cargo_success() -> Result<(), StrategyError> {
    let Some(current_exe) = current_exe_real_path() else {
        return Ok(());
    };
    let Some(cargo_bin) = cargo_bin_dir() else {
        return Ok(());
    };

    if !current_install_shadows_cargo_install(&current_exe, &cargo_bin) {
        return Ok(());
    }

    let report = super::uninstall::uninstall_path(&current_exe);
    match classify_shadow_cleanup(&report) {
        ShadowCleanupDecision::Removed => {
            eprintln!(
                "  · removed old non-cargo install at {} after cargo install",
                current_exe.display()
            );
            Ok(())
        }
        ShadowCleanupDecision::Scheduled => {
            // Windows: the running old copy can't be deleted in place, but a
            // helper will delete it on exit. The new cargo binary takes
            // precedence in a NEW shell. Warn honestly instead of claiming the
            // shadow is already gone.
            eprintln!(
                "  · WARNING: cargo install succeeded, but the old ND300 install at {} is still running and could not be removed in place.",
                current_exe.display()
            );
            eprintln!(
                "    It is scheduled for deletion when this process exits. The new cargo install in {} will take precedence in a NEW shell.",
                cargo_bin.display()
            );
            eprintln!(
                "    If `nd300 --version` still reports the old version in a new shell, remove the old path manually: {}",
                current_exe.display()
            );
            Ok(())
        }
        ShadowCleanupDecision::NotRemoved => Err(StrategyError::Fatal(format!(
            "cargo install succeeded, but the old ND300 install at {} could not be removed: {}",
            current_exe.display(),
            cleanup_notes(&report)
        ))),
    }
}

fn cleanup_current_install_for_cargo_retry() -> Result<(), StrategyError> {
    let Some(current_exe) = current_exe_real_path() else {
        return Ok(());
    };

    let report = super::uninstall::uninstall_path(&current_exe);
    match classify_shadow_cleanup(&report) {
        ShadowCleanupDecision::Removed => {
            eprintln!(
                "  · removed existing ND300 install at {} before retrying cargo install",
                current_exe.display()
            );
            Ok(())
        }
        // A scheduled-on-exit deletion is NOT good enough here: the immediate
        // cargo retry needs the file actually gone right now, but the helper
        // only deletes it after THIS process exits. Fail with instructions.
        ShadowCleanupDecision::Scheduled => Err(StrategyError::Fatal(format!(
            "cargo reported an existing nd300/speedqx binary, and removal of the current install at {} was only scheduled for process exit. \
             A cargo retry needs the file gone now — close this nd300 process and re-run `nd300 update`.",
            current_exe.display()
        ))),
        ShadowCleanupDecision::NotRemoved => Err(StrategyError::Fatal(format!(
            "cargo reported an existing nd300/speedqx binary, but the current install at {} could not be removed: {}",
            current_exe.display(),
            cleanup_notes(&report)
        ))),
    }
}

fn cleanup_notes(report: &super::uninstall::CleanupReport) -> String {
    if report.notes.is_empty() {
        "no additional details".to_string()
    } else {
        report.notes.join("; ")
    }
}

fn current_exe_real_path() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    Some(exe.canonicalize().unwrap_or(exe))
}

fn cargo_bin_dir() -> Option<PathBuf> {
    if let Some(home) = std::env::var_os("CARGO_HOME") {
        return Some(PathBuf::from(home).join("bin"));
    }

    #[cfg(windows)]
    {
        std::env::var_os("USERPROFILE").map(|home| PathBuf::from(home).join(".cargo").join("bin"))
    }

    #[cfg(not(windows))]
    {
        std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".cargo").join("bin"))
    }
}

fn current_install_shadows_cargo_install(current_exe: &Path, cargo_bin_dir: &Path) -> bool {
    if current_exe_looks_like_local_build(current_exe) {
        return false;
    }

    let Some(current_dir) = current_exe.parent() else {
        return false;
    };
    !same_path(current_dir, cargo_bin_dir)
}

fn current_exe_looks_like_local_build(current_exe: &Path) -> bool {
    current_exe
        .parent()
        .and_then(|dir| dir.file_name())
        .and_then(|name| name.to_str())
        .is_some_and(|name| matches!(name, "debug" | "release"))
        && current_exe
            .parent()
            .and_then(|dir| dir.parent())
            .and_then(|dir| dir.file_name())
            .and_then(|name| name.to_str())
            == Some("target")
}

fn same_path(left: &Path, right: &Path) -> bool {
    let left = left.canonicalize().unwrap_or_else(|_| left.to_path_buf());
    let right = right.canonicalize().unwrap_or_else(|_| right.to_path_buf());

    #[cfg(windows)]
    {
        left.to_string_lossy()
            .trim_end_matches(['\\', '/'])
            .eq_ignore_ascii_case(right.to_string_lossy().trim_end_matches(['\\', '/']))
    }

    #[cfg(not(windows))]
    {
        left == right
    }
}

fn cargo_failure_suggests_existing_binary_collision(message: &str) -> bool {
    let lower = message.to_ascii_lowercase();
    lower.contains("already exists") && (lower.contains("nd300") || lower.contains("speedqx"))
}

/// Run `rustup update stable` if rustup is on PATH. Best-effort: any failure is
/// non-fatal (we let the subsequent `cargo install` surface the real error).
/// Keeps the toolchain in lock-step with whatever Rust version nd300's MSRV
/// currently requires.
fn rustup_update_stable_best_effort() {
    if !tool_exists("rustup") {
        return;
    }
    println!("  Updating Rust toolchain (rustup update stable)…");
    let _ = Command::new("rustup").args(["update", "stable"]).status();
}

/// Spawn `launcher` with `args`, classify the outcome.
///
/// - `Err(NotFound)` → `Preflight` (launcher itself isn't on PATH; safe to fall through).
/// - `Err(other)` → `Preflight` (couldn't even spawn).
/// - non-zero exit → `Runtime` (ran but failed).
/// - zero exit → `Ok`.
fn run_command_status(launcher: &str, args: &[&str]) -> Result<(), StrategyError> {
    let res = Command::new(launcher).args(args).status();
    match res {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Err(StrategyError::Preflight(
            format!("{} not on PATH", launcher),
        )),
        Err(e) => Err(StrategyError::Preflight(format!(
            "Failed to spawn {}: {}",
            launcher, e
        ))),
        Ok(s) if s.success() => Ok(()),
        Ok(s) => Err(StrategyError::Runtime(format!(
            "{} exited with code {}",
            launcher,
            s.code().unwrap_or(-1)
        ))),
    }
}

fn run_command_capture(launcher: &str, args: &[&str]) -> Result<(), StrategyError> {
    let res = Command::new(launcher).args(args).output();
    match res {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Err(StrategyError::Preflight(
            format!("{} not on PATH", launcher),
        )),
        Err(e) => Err(StrategyError::Preflight(format!(
            "Failed to spawn {}: {}",
            launcher, e
        ))),
        Ok(output) if output.status.success() => Ok(()),
        Ok(output) => Err(StrategyError::Runtime(format!(
            "{} exited with code {}; {}",
            launcher,
            output.status.code().unwrap_or(-1),
            summarize_output(&output)
        ))),
    }
}

fn summarize_output(output: &Output) -> String {
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let combined = format!("{}{}", stdout, stderr).trim().to_string();
    if combined.is_empty() {
        return "no output".to_string();
    }

    const MAX_CHARS: usize = 4000;
    let chars: Vec<char> = combined.chars().collect();
    if chars.len() <= MAX_CHARS {
        combined
    } else {
        chars[chars.len() - MAX_CHARS..].iter().collect()
    }
}

#[cfg(unix)]
fn try_installer_curl() -> Result<(), StrategyError> {
    if !tool_exists("curl") {
        return Err(StrategyError::Preflight("curl not on PATH".into()));
    }
    // `set -e -o pipefail` so a curl HTTP/DNS/TLS error isn't silently swallowed
    // by `sh` exiting 0 on empty stdin. `-f` (--fail) makes curl exit non-zero on
    // HTTP >=400 instead of piping a 404 HTML body into sh.
    let script = format!(
        "set -e; set -o pipefail; curl --proto '=https' --tlsv1.2 -fLsS {} | sh",
        SHELL_INSTALLER
    );
    run_command_status("sh", &["-c", &script])
}

#[cfg(not(unix))]
fn try_installer_curl() -> Result<(), StrategyError> {
    Err(StrategyError::Preflight(
        "curl installer is Unix-only".into(),
    ))
}

#[cfg(unix)]
fn try_installer_wget() -> Result<(), StrategyError> {
    if !tool_exists("wget") {
        return Err(StrategyError::Preflight("wget not on PATH".into()));
    }
    let script = format!(
        "set -e; set -o pipefail; wget -qO- {} | sh",
        SHELL_INSTALLER
    );
    run_command_status("sh", &["-c", &script])
}

#[cfg(not(unix))]
fn try_installer_wget() -> Result<(), StrategyError> {
    Err(StrategyError::Preflight(
        "wget installer is Unix-only".into(),
    ))
}

#[cfg(windows)]
fn try_installer_powershell(launcher: &str) -> Result<(), StrategyError> {
    // `$ErrorActionPreference='Stop'` so an `irm` failure aborts the pipeline
    // rather than feeding nothing into iex (which would exit 0 and look like a
    // successful update).
    let script = format!("$ErrorActionPreference='Stop'; irm {} | iex", PS_INSTALLER);
    run_command_status(
        launcher,
        &[
            "-NoProfile",
            "-NonInteractive",
            "-ExecutionPolicy",
            "Bypass",
            "-Command",
            &script,
        ],
    )
}

#[cfg(not(windows))]
fn try_installer_powershell(_launcher: &str) -> Result<(), StrategyError> {
    Err(StrategyError::Preflight(
        "PowerShell installer is Windows-only".into(),
    ))
}

// ─── Windows MSI / EXE installer strategies (v3.1.0+) ─────────────────────────

/// msiexec exit code returned when the install completed successfully but a
/// reboot is required to finalize a file replacement (Windows Installer's Restart
/// Manager couldn't replace a locked file in-place and scheduled a delete-on-
/// reboot instead).
#[cfg(windows)]
const MSI_EXIT_REBOOT_REQUIRED: i32 = 3010;

/// Drive an async future to completion from synchronous code that is itself
/// running inside the outer (`#[tokio::main]`) runtime.
///
/// The MSI/EXE strategies are reached from the SYNCHRONOUS `execute_update`
/// helper, but `update::run` is `async` and already executing on a tokio worker
/// thread. Calling `Runtime::block_on` on a freshly-built runtime from that
/// worker would panic ("Cannot start a runtime from within a runtime"). So we
/// run the future on a dedicated OS thread (which is NOT in any runtime context)
/// that owns its own current-thread runtime, and join it. The future + output
/// must be `Send` to cross the thread boundary — reqwest's are.
#[cfg(windows)]
fn block_on<F>(fut: F) -> F::Output
where
    F: std::future::Future + Send,
    F::Output: Send,
{
    // `std::thread::scope` lets the spawned thread borrow non-'static data
    // (the reqwest future captures borrowed &str/&Path), and guarantees the
    // thread is joined before the scope returns.
    std::thread::scope(|scope| {
        scope
            .spawn(|| {
                tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .expect("failed to build current-thread runtime for installer download")
                    .block_on(fut)
            })
            .join()
            .expect("installer download thread panicked")
    })
}

/// Download a file from `url` to `path` over HTTPS using async reqwest. Used by
/// the MSI/EXE strategies to fetch the matching installer to `%TEMP%` before
/// launching it. TLS validation is enforced by reqwest's rustls backend; the
/// caller then re-fetches the `.sha256` sidecar and runs `verify_checksum` for
/// defense against a corporate-proxy interception or a tampered release asset.
#[cfg(windows)]
fn download_to_file(url: &str, path: &std::path::Path) -> Result<(), String> {
    block_on(async move {
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(120))
            .build()
            .map_err(|e| format!("HTTP client error: {}", e))?;
        let resp = client
            .get(url)
            .header("User-Agent", format!("nd300/{}", VERSION))
            .send()
            .await
            .map_err(|e| format!("Request failed: {}", e))?;
        if !resp.status().is_success() {
            return Err(format!("Download returned HTTP {}", resp.status()));
        }
        let bytes = resp
            .bytes()
            .await
            .map_err(|e| format!("Failed to read response body: {}", e))?;
        std::fs::write(path, &bytes)
            .map_err(|e| format!("Failed to write temp file {}: {}", path.display(), e))
    })
}

/// Fetch the cargo-dist `.sha256` sidecar at `<url>.sha256` and return the file
/// contents.
///
/// Format: `<lowercase-64-char-hex>  *<filename>` per cargo-dist's
/// `dist-manifest.json` generation and the parallel implementation in
/// `.github/workflows/windows-installers.yml`. Tolerant of trailing whitespace /
/// missing asterisk via `parse_sha256_sidecar`.
#[cfg(windows)]
fn fetch_sha256_sidecar(url: &str) -> Result<String, String> {
    let sidecar_url = format!("{}.sha256", url);
    block_on(async move {
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .build()
            .map_err(|e| format!("HTTP client error: {}", e))?;
        let resp = client
            .get(&sidecar_url)
            .header("User-Agent", format!("nd300/{}", VERSION))
            .send()
            .await
            .map_err(|e| format!("Sidecar request failed ({}): {}", sidecar_url, e))?;
        if !resp.status().is_success() {
            return Err(format!(
                "Sidecar fetch returned HTTP {} ({})",
                resp.status(),
                sidecar_url
            ));
        }
        resp.text()
            .await
            .map_err(|e| format!("Failed to read sidecar body: {}", e))
    })
}

/// Extract the 64-char hex from a `.sha256` sidecar line. Returns `None` when the
/// first whitespace-separated token is not exactly 64 hex characters.
#[cfg(windows)]
fn parse_sha256_sidecar(content: &str) -> Option<String> {
    content
        .split_whitespace()
        .next()
        .filter(|s| s.len() == 64 && s.chars().all(|c| c.is_ascii_hexdigit()))
        .map(|s| s.to_lowercase())
}

/// Compute the SHA-256 of `path`, returning the lowercase hex.
#[cfg(windows)]
fn compute_sha256(path: &std::path::Path) -> Result<String, String> {
    use sha2::{Digest, Sha256};
    let mut file = std::fs::File::open(path)
        .map_err(|e| format!("Failed to open {}: {}", path.display(), e))?;
    let mut hasher = Sha256::new();
    std::io::copy(&mut file, &mut hasher).map_err(|e| format!("Failed to hash: {}", e))?;
    Ok(format!("{:x}", hasher.finalize()))
}

/// Fetch the `.sha256` sidecar, compute the SHA-256 of the downloaded installer,
/// refuse to proceed on mismatch.
///
/// Defends against a network MITM that replaces the installer bytes in flight
/// (corporate TLS-interception proxies with a trusted root CA, hostile WiFi,
/// captive portals). The sidecar is fetched in a separate request — an attacker
/// would have to corrupt both the installer and the sidecar in a way that yields
/// a matching hash, which is preimage-hard.
#[cfg(windows)]
fn verify_checksum(installer_path: &std::path::Path, installer_url: &str) -> Result<(), String> {
    println!("  Verifying SHA256 checksum...");
    let sidecar_content = fetch_sha256_sidecar(installer_url)?;
    let expected = parse_sha256_sidecar(&sidecar_content).ok_or_else(|| {
        format!(
            "Malformed .sha256 sidecar from {}.sha256: {:?}",
            installer_url, sidecar_content
        )
    })?;
    let actual = compute_sha256(installer_path)?;
    checksum_verdict(&actual, &expected)
}

/// Compare a computed SHA-256 against the expected sidecar hash, refusing on
/// mismatch. Separated from the network fetch + file read in `verify_checksum` so
/// the load-bearing refusal-on-mismatch is unit-testable on any target.
#[cfg(any(windows, test))]
fn checksum_verdict(actual: &str, expected: &str) -> Result<(), String> {
    if actual.eq_ignore_ascii_case(expected) {
        Ok(())
    } else {
        Err(format!(
            "SHA256 mismatch — refusing to run installer.\n         Expected: {}\n         Got:      {}\n         This usually indicates a corrupted download or a network MITM.",
            expected, actual
        ))
    }
}

/// Whether the freshly-installed `--version` string matches the expected release
/// tag. Both sides are stripped of any prerelease/build-metadata suffix before
/// comparison, and an empty `installed` never matches (covers `--version`
/// failing to parse). Pure + platform-independent.
fn post_install_version_ok(installed: &str, expected: &str) -> bool {
    let installed_stripped = strip_prerelease_metadata(installed);
    let expected_stripped = strip_prerelease_metadata(expected);
    !installed_stripped.is_empty() && installed_stripped == expected_stripped
}

/// Re-exec the running binary with `--version` and return the parsed version (the
/// last whitespace token of `nd300 X.Y.Z` / `speedqx X.Y.Z`). `None` if the spawn
/// fails, the process errors, or the output doesn't parse. Cross-platform — used
/// by both the Windows installer verify and the cargo-path verify.
fn reexec_installed_version() -> Option<String> {
    let exe = std::env::current_exe().ok()?;
    let output = Command::new(&exe).arg("--version").output().ok()?;
    if !output.status.success() {
        return None;
    }
    let v = String::from_utf8_lossy(&output.stdout)
        .split_whitespace()
        .last()
        .unwrap_or("")
        .trim()
        .to_string();
    if v.is_empty() {
        None
    } else {
        Some(v)
    }
}

/// Confirm a `cargo install nd300 --force` update actually landed.
///
/// `cargo install` reports success (exit 0) even when crates.io still serves the
/// OLD version — a publish lag right after a GitHub release, or a failed publish
/// run. Without this check, the updater would print "Updated to vX" while
/// `nd300 --version` is unchanged, then loop forever. Re-exec `--version`; on
/// mismatch return a `Runtime` error so `execute_update` falls through to the
/// prebuilt GitHub-release installer, which always carries `latest`.
fn verify_cargo_post_install(expected: &str) -> Result<(), StrategyError> {
    match reexec_installed_version() {
        Some(installed) if post_install_version_ok(&installed, expected) => Ok(()),
        Some(installed) => Err(StrategyError::Runtime(format!(
            "cargo install reported success but `nd300 --version` still reports v{installed} (expected v{expected}). crates.io may not have v{expected} yet, or another nd300 is earlier on your PATH — falling through to the prebuilt installer."
        ))),
        None => Err(StrategyError::Runtime(
            "cargo install reported success but `nd300 --version` could not be run to confirm the new version — falling through to the prebuilt installer.".to_string(),
        )),
    }
}

/// After the installer reports success, re-exec `<current_exe> --version` and
/// confirm the on-disk binary has been updated. Catches the case where the
/// installer exits 0 but the file replacement didn't take effect (Restart
/// Manager edge cases, MSI re-running the SAME version, etc.).
#[cfg(windows)]
fn verify_post_install(expected: &str) -> Result<(), String> {
    let installed = reexec_installed_version()
        .ok_or_else(|| "Failed to run `nd300 --version` to confirm the install".to_string())?;
    if post_install_version_ok(&installed, expected) {
        Ok(())
    } else {
        Err(format!(
            "Installer exited successfully but `nd300 --version` still reports v{} (expected v{}). The installed binary may be locked by another process — close other nd300/speedqx windows / shells and re-run, or reboot to let Windows finish a deferred file replace.",
            installed, expected
        ))
    }
}

/// Download the matching MSI, verify its SHA256, re-run it via
/// `msiexec /i /passive /norestart`, then re-exec the binary with `--version` to
/// confirm the file replacement actually took effect.
///
/// WiX `MajorUpgrade` in wix/main.wxs and wix-corporate/corporate.wxs handles the
/// uninstall-old-then-install-new step atomically. Windows Installer's Restart
/// Manager handles the "binary in use" case by renaming the locked file (the
/// running nd300 process keeps its open file handle to the OLD inode); if RM
/// falls back to delete-on-reboot, msiexec returns 3010 and we surface that
/// without claiming success.
#[cfg(windows)]
fn try_msi_install(url: &str, latest: &str) -> Result<(), StrategyError> {
    let temp_path = std::env::temp_dir().join(format!("nd300-update-{}.msi", latest));

    let result = (|| -> Result<(), StrategyError> {
        println!("  Downloading MSI installer...");
        download_to_file(url, &temp_path)
            .map_err(|e| StrategyError::Runtime(format!("Download failed: {}", e)))?;

        verify_checksum(&temp_path, url).map_err(StrategyError::Runtime)?;

        println!("  Launching Windows Installer...");
        // /passive shows a progress dialog with no user interaction; /norestart
        // suppresses any reboot prompt. For the Global perMachine MSI, msiexec
        // triggers UAC; for the Corporate perUser MSI, it installs silently into
        // LocalAppData with no elevation prompt.
        let status = Command::new("msiexec")
            .args(["/i", &temp_path.to_string_lossy(), "/passive", "/norestart"])
            .status()
            .map_err(|e| StrategyError::Preflight(format!("Failed to spawn msiexec: {}", e)))?;

        let code = status.code().unwrap_or(-1);
        if code == MSI_EXIT_REBOOT_REQUIRED {
            return Err(StrategyError::Runtime(format!(
                "MSI install completed but requires a reboot to finalize (msiexec exit {}). Reboot, then verify with `nd300 --version`.",
                MSI_EXIT_REBOOT_REQUIRED
            )));
        }
        if !status.success() {
            return Err(StrategyError::Runtime(format!(
                "msiexec exited with code {} (likely user cancel, UAC denied, or install error)",
                code
            )));
        }

        verify_post_install(latest).map_err(StrategyError::Runtime)?;
        Ok(())
    })();

    // Best-effort cleanup — never alters the result. (msiexec has already exited,
    // so the file is no longer in use.)
    let _ = std::fs::remove_file(&temp_path);
    result
}

/// Download the matching Inno Setup EXE installer, verify its SHA256, re-run it
/// with `/SILENT /SUPPRESSMSGBOXES /NORESTART`, then verify the post-install
/// version.
///
/// Inno Setup's AppId-based upgrade detection silently uninstalls the old version
/// before installing the new one. For the Global perMachine EXE,
/// `PrivilegesRequired=admin` in inno/global.iss triggers UAC; the Corporate
/// perUser EXE (`PrivilegesRequired=lowest`) installs without elevation.
#[cfg(windows)]
fn try_exe_install(url: &str, latest: &str) -> Result<(), StrategyError> {
    let temp_path = std::env::temp_dir().join(format!("nd300-update-{}-setup.exe", latest));

    let result = (|| -> Result<(), StrategyError> {
        println!("  Downloading EXE installer...");
        download_to_file(url, &temp_path)
            .map_err(|e| StrategyError::Runtime(format!("Download failed: {}", e)))?;

        verify_checksum(&temp_path, url).map_err(StrategyError::Runtime)?;

        println!("  Launching Inno Setup installer...");
        let status = Command::new(&temp_path)
            .args(["/SILENT", "/SUPPRESSMSGBOXES", "/NORESTART"])
            .status()
            .map_err(|e| {
                StrategyError::Preflight(format!("Failed to spawn EXE installer: {}", e))
            })?;

        if !status.success() {
            return Err(StrategyError::Runtime(format!(
                "EXE installer exited with code {} (likely user cancel, UAC denied, or install error)",
                status.code().unwrap_or(-1)
            )));
        }

        verify_post_install(latest).map_err(StrategyError::Runtime)?;
        Ok(())
    })();

    let _ = std::fs::remove_file(&temp_path);
    result
}

#[cfg(not(windows))]
fn try_msi_install(_url: &str, _latest: &str) -> Result<(), StrategyError> {
    Err(StrategyError::Preflight(
        "MSI installer is Windows-only".into(),
    ))
}

#[cfg(not(windows))]
fn try_exe_install(_url: &str, _latest: &str) -> Result<(), StrategyError> {
    Err(StrategyError::Preflight(
        "EXE installer is Windows-only".into(),
    ))
}

// ─── Windows install-origin detection (v3.1.0+) ──────────────────────────────

/// Read the `HKCU\Software\ND300\InstallSource` registry value written by the
/// four first-class installers on install. Authoritative when present. Returns
/// `None` if the key is missing, the value is missing, the value type isn't a
/// string, or the value content doesn't match a known variant.
#[cfg(windows)]
fn read_install_source_marker() -> Option<InstallOrigin> {
    use winreg::enums::HKEY_CURRENT_USER;
    use winreg::RegKey;

    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    let key = hkcu.open_subkey("Software\\ND300").ok()?;
    let value: String = key.get_value("InstallSource").ok()?;
    match value.as_str() {
        "msi-global" => Some(InstallOrigin::MsiGlobal),
        "msi-corporate" => Some(InstallOrigin::MsiCorporate),
        "exe-global" => Some(InstallOrigin::ExeGlobal),
        "exe-corporate" => Some(InstallOrigin::ExeCorporate),
        _ => None,
    }
}

/// Determine how this binary was installed on Windows.
///
/// 1. Read the `HKCU\Software\ND300\InstallSource` marker. Authoritative when
///    present. All four first-class installers write this marker on install.
/// 2. If no marker, fall back to path-based detection on the running binary's
///    location. Handles the cargo install / PowerShell installer path (which
///    doesn't write a marker), and any pre-marker legacy installs.
///
/// The path fallback maps Program Files → `MsiGlobal` and LocalAppData\Programs
/// → `MsiCorporate` (it can't distinguish MSI vs EXE when the marker is missing
/// because both installer formats target the same paths within each edition —
/// that's by design). When the marker IS present, the EXE vs MSI distinction is
/// preserved.
#[cfg(windows)]
fn detect_install_origin() -> InstallOrigin {
    if let Some(origin) = read_install_source_marker() {
        return origin;
    }

    let Ok(exe) = std::env::current_exe() else {
        return InstallOrigin::Unknown;
    };
    classify_install_path(&exe.to_string_lossy())
}

/// Pure-function half of `detect_install_origin()` for unit testing. Lowercased
/// substring match handles drive-letter casing and Windows path
/// case-insensitivity. Order matters: check more-specific paths first.
///
/// LOCKSTEP: these substrings must match the install dirs in wix/main.wxs
/// (Program Files\nd300), wix-corporate/corporate.wxs + inno/corporate.iss
/// (LocalAppData\Programs\nd300), and the cargo bin (`.cargo\bin`).
#[cfg(windows)]
fn classify_install_path(exe_path: &str) -> InstallOrigin {
    let lower = exe_path.to_lowercase();
    if lower.contains("\\program files\\nd300\\") {
        InstallOrigin::MsiGlobal
    } else if lower.contains("\\appdata\\local\\programs\\nd300\\") {
        InstallOrigin::MsiCorporate
    } else if lower.contains("\\.cargo\\bin\\") {
        InstallOrigin::CargoOrInstaller
    } else {
        InstallOrigin::Unknown
    }
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unix_with_cargo_orders_cargo_first() {
        assert_eq!(
            order_strategies(true, TargetOs::Unix),
            vec![
                UpdateStrategy::Cargo,
                UpdateStrategy::InstallerCurl,
                UpdateStrategy::InstallerWget,
            ]
        );
    }

    #[test]
    fn unix_without_cargo_prunes_cargo() {
        assert_eq!(
            order_strategies(false, TargetOs::Unix),
            vec![UpdateStrategy::InstallerCurl, UpdateStrategy::InstallerWget,]
        );
    }

    #[test]
    fn windows_with_cargo_orders_cargo_first() {
        assert_eq!(
            order_strategies(true, TargetOs::Windows),
            vec![
                UpdateStrategy::Cargo,
                UpdateStrategy::InstallerPowerShell,
                UpdateStrategy::InstallerPwsh,
            ]
        );
    }

    #[test]
    fn windows_without_cargo_prunes_cargo() {
        assert_eq!(
            order_strategies(false, TargetOs::Windows),
            vec![
                UpdateStrategy::InstallerPowerShell,
                UpdateStrategy::InstallerPwsh,
            ]
        );
    }

    #[test]
    fn json_method_maps_to_legacy_taxonomy() {
        assert_eq!(UpdateStrategy::Cargo.json_method(), "cargo");
        assert_eq!(UpdateStrategy::InstallerCurl.json_method(), "installer");
        assert_eq!(UpdateStrategy::InstallerWget.json_method(), "installer");
        assert_eq!(
            UpdateStrategy::InstallerPowerShell.json_method(),
            "installer"
        );
        assert_eq!(UpdateStrategy::InstallerPwsh.json_method(), "installer");
        // New v3.1.0+ MSI/EXE strategies map to "installer" in the legacy
        // `method` field. External consumers wanting the specific installer type
        // should read the precise `strategy` field instead.
        assert_eq!(UpdateStrategy::MsiGlobal.json_method(), "installer");
        assert_eq!(UpdateStrategy::MsiCorporate.json_method(), "installer");
        assert_eq!(UpdateStrategy::ExeGlobal.json_method(), "installer");
        assert_eq!(UpdateStrategy::ExeCorporate.json_method(), "installer");
    }

    #[test]
    fn new_strategies_have_stable_json_ids() {
        // These IDs are part of the public JSON contract; renaming them is a
        // schema break. Keep them in lockstep with the README Self-Update
        // section and CLAUDE.md / AGENTS.md.
        assert_eq!(UpdateStrategy::MsiGlobal.json_id(), "msi_global");
        assert_eq!(UpdateStrategy::MsiCorporate.json_id(), "msi_corporate");
        assert_eq!(UpdateStrategy::ExeGlobal.json_id(), "exe_global");
        assert_eq!(UpdateStrategy::ExeCorporate.json_id(), "exe_corporate");
    }

    #[test]
    fn new_strategies_have_distinct_labels() {
        // Labels feed into the text-mode "Updating via X..." line. Verify each is
        // distinct (otherwise users can't tell which installer is downloading
        // from text output alone).
        let labels = [
            UpdateStrategy::MsiGlobal.label(),
            UpdateStrategy::MsiCorporate.label(),
            UpdateStrategy::ExeGlobal.label(),
            UpdateStrategy::ExeCorporate.label(),
            UpdateStrategy::Cargo.label(),
            UpdateStrategy::InstallerPowerShell.label(),
            UpdateStrategy::InstallerPwsh.label(),
            UpdateStrategy::InstallerCurl.label(),
            UpdateStrategy::InstallerWget.label(),
        ];
        let unique: std::collections::HashSet<_> = labels.iter().collect();
        assert_eq!(
            unique.len(),
            labels.len(),
            "all strategy labels must be unique"
        );
    }

    #[test]
    fn is_newer_basic() {
        assert!(is_newer("3.0.0", "3.0.1"));
        assert!(is_newer("2.9.0", "3.0.0"));
        assert!(!is_newer("3.0.1", "3.0.0"));
        assert!(!is_newer("3.0.1", "3.0.1"));
        assert!(is_newer("3.0", "3.0.1"));
    }

    #[test]
    fn is_newer_different_lengths() {
        assert!(is_newer("1.0", "1.0.1"));
        assert!(!is_newer("1.0.1", "1.0"));
    }

    #[test]
    fn is_newer_major_versions() {
        assert!(is_newer("3.8.0", "4.0.0"));
        assert!(!is_newer("4.0.0", "3.99.99"));
    }

    #[test]
    fn cargo_failure_detection_catches_existing_nd300_or_speedqx_binary() {
        assert!(cargo_failure_suggests_existing_binary_collision(
            "error: binary `nd300` already exists in destination"
        ));
        assert!(cargo_failure_suggests_existing_binary_collision(
            "error: failed to install binary as `speedqx` because it already exists"
        ));
        assert!(!cargo_failure_suggests_existing_binary_collision(
            "error: failed to compile dependency"
        ));
    }

    #[test]
    fn cargo_success_cleans_only_shadowing_non_cargo_current_install() {
        assert!(current_install_shadows_cargo_install(
            std::path::Path::new("/usr/local/bin/nd300"),
            std::path::Path::new("/home/alice/.cargo/bin"),
        ));
        assert!(!current_install_shadows_cargo_install(
            std::path::Path::new("/repo/target/debug/nd300"),
            std::path::Path::new("/home/alice/.cargo/bin"),
        ));
        assert!(!current_install_shadows_cargo_install(
            std::path::Path::new("/home/alice/.cargo/bin/nd300"),
            std::path::Path::new("/home/alice/.cargo/bin"),
        ));
    }

    fn cleanup_report(removed: bool, scheduled: bool) -> super::super::uninstall::CleanupReport {
        super::super::uninstall::CleanupReport {
            binary_removed: removed,
            binary_removal_scheduled: scheduled,
            sibling_removed: false,
            receipt_removed: false,
            path_cleaned: false,
            notes: Vec::new(),
        }
    }

    #[test]
    fn shadow_cleanup_decision_maps_report_to_outcome() {
        // Actually gone → clean success.
        assert_eq!(
            classify_shadow_cleanup(&cleanup_report(true, false)),
            ShadowCleanupDecision::Removed
        );
        // Windows scheduled-on-exit → honest warning (Ok at the call site), not Fatal.
        assert_eq!(
            classify_shadow_cleanup(&cleanup_report(false, true)),
            ShadowCleanupDecision::Scheduled
        );
        // Neither → Fatal.
        assert_eq!(
            classify_shadow_cleanup(&cleanup_report(false, false)),
            ShadowCleanupDecision::NotRemoved
        );
        // `binary_removed` wins even if both happen to be set.
        assert_eq!(
            classify_shadow_cleanup(&cleanup_report(true, true)),
            ShadowCleanupDecision::Removed
        );
    }

    // ── Ported TR-300 capability tests ───────────────────────────────────────

    #[test]
    fn strip_prerelease_metadata_handles_prereleases() {
        assert_eq!(strip_prerelease_metadata("3.1.2-rc.1"), "3.1.2");
        assert_eq!(strip_prerelease_metadata("3.1.2-alpha.10"), "3.1.2");
    }

    #[test]
    fn strip_prerelease_metadata_handles_build_metadata() {
        assert_eq!(strip_prerelease_metadata("3.1.2+nightly.42"), "3.1.2");
        assert_eq!(strip_prerelease_metadata("3.1.2+sha.abc123"), "3.1.2");
    }

    #[test]
    fn strip_prerelease_metadata_leaves_clean_version_alone() {
        assert_eq!(strip_prerelease_metadata("3.1.2"), "3.1.2");
        assert_eq!(strip_prerelease_metadata("1.0"), "1.0");
        assert_eq!(strip_prerelease_metadata(""), "");
    }

    #[test]
    fn is_newer_treats_prerelease_of_higher_triple_as_newer() {
        assert!(is_newer("3.1.1", "3.1.2-rc.1"));
    }

    #[test]
    fn is_newer_treats_stable_as_newer_than_prerelease_of_same_triple() {
        assert!(is_newer("3.1.2-rc.1", "3.1.2"));
    }

    #[test]
    fn is_newer_treats_two_prereleases_of_same_triple_as_equal() {
        assert!(!is_newer("3.1.2-rc.1", "3.1.2-rc.2"));
    }

    #[test]
    fn is_newer_handles_build_metadata_as_equal_when_triples_match() {
        assert!(!is_newer("3.1.2", "3.1.2+sha.deadbeef"));
        assert!(!is_newer("3.1.2+nightly.41", "3.1.2+nightly.42"));
    }

    #[test]
    fn post_install_version_ok_matches_after_stripping() {
        // Exact match — the happy path after a successful in-place upgrade.
        assert!(post_install_version_ok("3.1.0", "3.1.0"));
        // Installed string carries a build-metadata suffix the release tag lacks.
        assert!(post_install_version_ok("3.1.0+sha.abc123", "3.1.0"));
        // Mismatch — installer exited 0 but the file replacement didn't take.
        assert!(!post_install_version_ok("3.0.11", "3.1.0"));
        // Empty installed string (e.g. `--version` output failed to parse).
        assert!(!post_install_version_ok("", "3.1.0"));
    }

    #[test]
    fn post_install_version_ok_drives_cargo_verify_logic() {
        // The cargo-path verify (verify_cargo_post_install) treats a matching
        // re-exec version as success and a stale one as fall-through; the decision
        // is post_install_version_ok, re-pinned here for the cargo path
        // specifically (crates.io lag → installed stays old).
        assert!(post_install_version_ok("3.1.0", "3.1.0"));
        assert!(!post_install_version_ok("3.0.11", "3.1.0"));
    }

    #[test]
    fn checksum_verdict_accepts_match_and_refuses_mismatch() {
        let hash = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";
        // Exact match passes.
        assert!(checksum_verdict(hash, hash).is_ok());
        // Case-insensitive match passes (sidecars may be upper/lower case).
        assert!(checksum_verdict(&hash.to_uppercase(), hash).is_ok());
        // A mismatch is REFUSED — this is the load-bearing MITM/corruption guard,
        // so it must never silently pass.
        let err = checksum_verdict("deadbeef", hash).unwrap_err();
        assert!(err.contains("SHA256 mismatch"), "err: {err}");
    }

    #[test]
    fn http_status_message_explains_rate_limit() {
        // 403 with the rate-limit-exhausted header → the explicit message.
        assert!(http_status_message(403, Some("0")).contains("rate limit"));
        // 403 without the exhausted header → generic HTTP message.
        assert!(http_status_message(403, None).contains("HTTP 403"));
        // Other error codes → generic HTTP message.
        assert!(http_status_message(500, None).contains("HTTP 500"));
    }

    #[cfg(windows)]
    #[test]
    fn parse_sha256_sidecar_accepts_cargo_dist_format() {
        // cargo-dist publishes lines like "<hex>  *<filename>" (two spaces,
        // asterisk-prefixed name) — see windows-installers.yml's sidecar step.
        let line = "abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789  *nd300-x86_64-pc-windows-msvc.msi";
        assert_eq!(
            parse_sha256_sidecar(line).as_deref(),
            Some("abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789"),
        );
    }

    #[cfg(windows)]
    #[test]
    fn parse_sha256_sidecar_accepts_no_asterisk_variant() {
        let line = "1234567890abcdef1234567890abcdef1234567890abcdef1234567890abcdef nd300.msi";
        assert_eq!(
            parse_sha256_sidecar(line).as_deref(),
            Some("1234567890abcdef1234567890abcdef1234567890abcdef1234567890abcdef"),
        );
    }

    #[cfg(windows)]
    #[test]
    fn parse_sha256_sidecar_normalizes_to_lowercase() {
        let line = "ABCDEF0123456789ABCDEF0123456789ABCDEF0123456789ABCDEF0123456789  *foo.msi";
        assert_eq!(
            parse_sha256_sidecar(line).as_deref(),
            Some("abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789"),
        );
    }

    #[cfg(windows)]
    #[test]
    fn parse_sha256_sidecar_rejects_wrong_length() {
        assert_eq!(parse_sha256_sidecar("abcdef  *foo.msi"), None);
        let too_long = format!("{}0  *foo.msi", "a".repeat(64));
        assert_eq!(parse_sha256_sidecar(&too_long), None);
        assert_eq!(parse_sha256_sidecar(""), None);
    }

    #[cfg(windows)]
    #[test]
    fn parse_sha256_sidecar_rejects_non_hex_chars() {
        let bad = format!("{}  *foo.msi", "g".repeat(64));
        assert_eq!(parse_sha256_sidecar(&bad), None);
    }

    #[cfg(windows)]
    #[test]
    fn compute_sha256_matches_known_value() {
        // Empty file → known SHA256 of the empty input.
        let dir = std::env::temp_dir().join(format!("nd300-update-tests-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("empty.bin");
        std::fs::write(&path, b"").unwrap();
        let hash = compute_sha256(&path).unwrap();
        assert_eq!(
            hash,
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(windows)]
    #[test]
    fn install_origin_classify_program_files_is_msi_global() {
        assert_eq!(
            classify_install_path(r"C:\Program Files\nd300\bin\nd300.exe"),
            InstallOrigin::MsiGlobal,
        );
        // Case-insensitive: drive letter or "Program Files" capitalization
        // shouldn't change the verdict.
        assert_eq!(
            classify_install_path(r"c:\PROGRAM FILES\nd300\BIN\nd300.exe"),
            InstallOrigin::MsiGlobal,
        );
    }

    #[cfg(windows)]
    #[test]
    fn install_origin_classify_localappdata_is_msi_corporate() {
        assert_eq!(
            classify_install_path(r"C:\Users\alice\AppData\Local\Programs\nd300\bin\nd300.exe"),
            InstallOrigin::MsiCorporate,
        );
    }

    #[cfg(windows)]
    #[test]
    fn install_origin_classify_cargo_bin_is_cargo_or_installer() {
        assert_eq!(
            classify_install_path(r"C:\Users\alice\.cargo\bin\nd300.exe"),
            InstallOrigin::CargoOrInstaller,
        );
    }

    #[cfg(windows)]
    #[test]
    fn install_origin_classify_random_path_is_unknown() {
        assert_eq!(
            classify_install_path(r"D:\portable\nd300\nd300.exe"),
            InstallOrigin::Unknown,
        );
        assert_eq!(
            classify_install_path(r"C:\Users\alice\Downloads\nd300.exe"),
            InstallOrigin::Unknown,
        );
    }

    #[cfg(windows)]
    #[test]
    fn install_origin_json_ids_are_kebab_case() {
        // The JSON id matches the literal registry marker value written by the
        // installers. LOCKSTEP with wix/main.wxs, wix-corporate/corporate.wxs,
        // inno/global.iss, inno/corporate.iss.
        assert_eq!(InstallOrigin::MsiGlobal.json_id(), "msi-global");
        assert_eq!(InstallOrigin::MsiCorporate.json_id(), "msi-corporate");
        assert_eq!(InstallOrigin::ExeGlobal.json_id(), "exe-global");
        assert_eq!(InstallOrigin::ExeCorporate.json_id(), "exe-corporate");
        assert_eq!(
            InstallOrigin::CargoOrInstaller.json_id(),
            "cargo-or-installer"
        );
        assert_eq!(InstallOrigin::Unknown.json_id(), "unknown");
    }
}
