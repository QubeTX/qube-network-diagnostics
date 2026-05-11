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

const MANUAL_INSTALL_URL: &str = "https://github.com/QubeTX/qube-network-diagnostics#install";

// ─── Strategy types ──────────────────────────────────────────────────────────

/// Ordered candidate strategies for updating the binary. The runner tries each
/// in order and falls through on any failure (preflight or runtime) until one
/// succeeds. Cargo-first when available, cargo-dist installer as universal
/// fallback.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum UpdateStrategy {
    Cargo,
    InstallerCurl,
    InstallerWget,
    InstallerPowerShell,
    InstallerPwsh,
}

impl UpdateStrategy {
    fn label(self) -> &'static str {
        match self {
            UpdateStrategy::Cargo => "cargo install",
            UpdateStrategy::InstallerCurl => "curl shell installer",
            UpdateStrategy::InstallerWget => "wget shell installer",
            UpdateStrategy::InstallerPowerShell => "PowerShell installer",
            UpdateStrategy::InstallerPwsh => "pwsh installer",
        }
    }

    fn json_id(self) -> &'static str {
        match self {
            UpdateStrategy::Cargo => "cargo",
            UpdateStrategy::InstallerCurl => "installer_curl",
            UpdateStrategy::InstallerWget => "installer_wget",
            UpdateStrategy::InstallerPowerShell => "installer_powershell",
            UpdateStrategy::InstallerPwsh => "installer_pwsh",
        }
    }

    /// Backward-compat label for the JSON `"method"` field on success.
    fn json_method(self) -> &'static str {
        if matches!(self, UpdateStrategy::Cargo) {
            "cargo"
        } else {
            "installer"
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
    /// The strategy ran end-to-end but reported a non-zero exit. cargo and
    /// cargo-dist installers are both atomic (write-to-temp, move-into-place),
    /// so it's still safe to fall through to the next strategy.
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

    match execute_update(&strategies) {
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
            let output = serde_json::json!({
                "action": "update",
                "success": false,
                "message": format!("Failed to check for updates: {}", e),
                "current_version": VERSION,
            });
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
        let output = serde_json::json!({
            "action": "update",
            "success": true,
            "message": "Already on the latest version",
            "current_version": current,
            "latest_version": latest,
            "update_available": false,
        });
        println!(
            "{}",
            serde_json::to_string_pretty(&output).unwrap_or_else(|_| "{}".to_string())
        );
        return 0;
    }

    let strategies = build_strategy_list();
    match execute_update(&strategies) {
        Ok(used) => {
            let output = serde_json::json!({
                "action": "update",
                "success": true,
                "message": format!("Updated from v{} to v{}", current, latest),
                "current_version": current,
                "latest_version": latest,
                "update_available": true,
                "method": used.json_method(),
                "strategy": used.json_id(),
            });
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
            let output = serde_json::json!({
                "action": "update",
                "success": false,
                "message": "Update failed; see attempts",
                "current_version": current,
                "latest_version": latest,
                "update_available": true,
                "attempts": attempts,
            });
            println!(
                "{}",
                serde_json::to_string_pretty(&output).unwrap_or_else(|_| "{}".to_string())
            );
            2
        }
    }
}

// ─── Version probing ─────────────────────────────────────────────────────────

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
        return Err(format!("GitHub API returned status {}", resp.status()));
    }

    let body: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| format!("Failed to parse response: {}", e))?;

    let tag = body["tag_name"]
        .as_str()
        .ok_or("Missing tag_name in response")?;

    Ok(tag.strip_prefix('v').unwrap_or(tag).to_string())
}

fn is_newer(current: &str, latest: &str) -> bool {
    let parse =
        |v: &str| -> Vec<u64> { v.split('.').filter_map(|s| s.parse::<u64>().ok()).collect() };

    let c = parse(current);
    let l = parse(latest);

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
    false
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
    order_strategies(cargo_invokable(), current_target_os())
}

// ─── Strategy execution ──────────────────────────────────────────────────────

fn execute_update(strategies: &[UpdateStrategy]) -> Result<UpdateStrategy, UpdateFailure> {
    let mut attempts = Vec::new();
    for &strategy in strategies {
        match try_strategy(strategy) {
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

fn try_strategy(strategy: UpdateStrategy) -> Result<(), StrategyError> {
    match strategy {
        UpdateStrategy::Cargo => try_cargo_install(),
        UpdateStrategy::InstallerCurl => try_installer_curl(),
        UpdateStrategy::InstallerWget => try_installer_wget(),
        UpdateStrategy::InstallerPowerShell => try_installer_powershell("powershell"),
        UpdateStrategy::InstallerPwsh => try_installer_powershell("pwsh"),
    }
}

fn try_cargo_install() -> Result<(), StrategyError> {
    match run_command_capture("cargo", &["install", "nd300", "--force"]) {
        Ok(()) => cleanup_shadowing_current_install_after_cargo_success(),
        Err(StrategyError::Runtime(msg))
            if cargo_failure_suggests_existing_binary_collision(&msg) =>
        {
            cleanup_current_install_for_cargo_retry()?;
            run_command_capture("cargo", &["install", "nd300", "--force"])
        }
        other => other,
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
    if report.binary_removed {
        eprintln!(
            "  · removed old non-cargo install at {} after cargo install",
            current_exe.display()
        );
        return Ok(());
    }

    Err(StrategyError::Fatal(format!(
        "cargo install succeeded, but the old ND300 install at {} could not be removed: {}",
        current_exe.display(),
        cleanup_notes(&report)
    )))
}

fn cleanup_current_install_for_cargo_retry() -> Result<(), StrategyError> {
    let Some(current_exe) = current_exe_real_path() else {
        return Ok(());
    };

    let report = super::uninstall::uninstall_path(&current_exe);
    if report.binary_removed {
        eprintln!(
            "  · removed existing ND300 install at {} before retrying cargo install",
            current_exe.display()
        );
        return Ok(());
    }

    Err(StrategyError::Fatal(format!(
        "cargo reported an existing nd300/speedqx binary, but the current install at {} could not be removed: {}",
        current_exe.display(),
        cleanup_notes(&report)
    )))
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
    // `set -e -o pipefail` so a curl HTTP/DNS/TLS error isn't silently
    // swallowed by `sh` exiting 0 on empty stdin. `-f` (--fail) makes curl
    // exit non-zero on HTTP >=400 instead of piping a 404 HTML body into sh.
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
    // rather than feeding nothing into iex (which would exit 0 and look like
    // a successful update).
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
}
