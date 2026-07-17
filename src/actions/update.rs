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
#[cfg(windows)]
use std::process::{Command, Output, Stdio};

use super::{fail_icon, success_icon};

/// GitHub API endpoint for the latest release.
const RELEASES_URL: &str =
    "https://api.github.com/repos/QubeTX/qube-network-diagnostics/releases/latest";

/// Users invoke versionless `latest` URLs, but after the API resolves one
/// release every internal download is pinned to that immutable tag.
const RELEASE_DOWNLOAD_BASE: &str =
    "https://github.com/QubeTX/qube-network-diagnostics/releases/download";
#[cfg(windows)]
const PS_INSTALLER_ASSET: &str = "nd300-installer.ps1";
const MSI_GLOBAL_ASSET: &str = "nd300-x86_64-pc-windows-msvc.msi";
const MSI_CORPORATE_ASSET: &str = "nd300-x86_64-pc-windows-msvc-corporate.msi";
const EXE_GLOBAL_ASSET: &str = "nd300-x86_64-pc-windows-msvc-setup.exe";
const EXE_CORPORATE_ASSET: &str = "nd300-x86_64-pc-windows-msvc-corporate-setup.exe";

const MANUAL_INSTALL_URL: &str = "https://reports.qubetx.com/nd300#install";

// ─── Strategy types ──────────────────────────────────────────────────────────

/// Ordered candidate strategies for updating the binary.
///
/// A proven installation owner gets only strategies that preserve that owner:
/// Cargo stays Cargo, a receipt-proven Unix archive stays a verified archive,
/// and the Windows MSI/EXE variants stay the same format, edition, and scope.
/// The only multi-entry plan is the legacy Windows standalone installer, where
/// Windows PowerShell and pwsh execute the same signed installer script.
///
/// For first-class Windows installs (v3.1.0+), `build_strategy_list` returns a
/// *single* matching MSI/EXE strategy (no cross-fall-back between installer
/// types — re-running a different product would create coexistence problems:
/// two ARP entries, PATH ordering decides which wins).
// The four MSI/EXE variants are only ever constructed inside the #[cfg(windows)]
// block of build_strategy_list(), so on non-Windows targets the dead_code lint
// flags them as never-constructed. The variants still need to exist on every
// platform so the label()/json_id()/json_method() match arms stay exhaustive and
// the try_strategy() dispatch arms compile. UnixArchive is absent on Windows so
// the Windows build keeps dead-code linting strict for every reachable route.
#[cfg_attr(not(windows), allow(dead_code))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum UpdateStrategy {
    Cargo,
    #[cfg(unix)]
    UnixArchive,
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
            #[cfg(unix)]
            UpdateStrategy::UnixArchive => "verified release archive",
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
            #[cfg(unix)]
            UpdateStrategy::UnixArchive => "unix_archive",
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
pub(crate) enum InstallOrigin {
    /// `C:\Program Files\nd300\bin\nd300.exe`, installed from wix/main.wxs.
    MsiGlobal,
    /// `%LocalAppData%\Programs\nd300\bin\nd300.exe`, from wix-corporate/corporate.wxs.
    MsiCorporate,
    /// `C:\Program Files\nd300\bin\nd300.exe`, from inno/global.iss.
    ExeGlobal,
    /// `%LocalAppData%\Programs\nd300\bin\nd300.exe`, from inno/corporate.iss.
    ExeCorporate,
    /// `~\.cargo\bin\nd300.exe` — either Cargo-registered or installed by the
    /// versionless cargo-dist PowerShell installer. Registry metadata selects
    /// exactly one of those two same-channel routes.
    CargoOrInstaller,
    /// Couldn't determine origin (custom install location, portable use, etc.).
    /// Updates stop safely rather than guessing an owner or changing channels.
    Unknown,
}

#[cfg(windows)]
impl InstallOrigin {
    /// String form for JSON output. Matches the registry marker values written
    /// by the installers; `cargo-or-installer` / `unknown` are synthesized by the
    /// path-based fallback.
    pub(crate) fn json_id(self) -> &'static str {
        match self {
            InstallOrigin::MsiGlobal => "msi-global",
            InstallOrigin::MsiCorporate => "msi-corporate",
            InstallOrigin::ExeGlobal => "exe-global",
            InstallOrigin::ExeCorporate => "exe-corporate",
            InstallOrigin::CargoOrInstaller => "cargo-or-installer",
            InstallOrigin::Unknown => "unknown",
        }
    }

    fn from_marker(value: &str) -> Option<Self> {
        match value {
            "msi-global" => Some(InstallOrigin::MsiGlobal),
            "msi-corporate" => Some(InstallOrigin::MsiCorporate),
            "exe-global" => Some(InstallOrigin::ExeGlobal),
            "exe-corporate" => Some(InstallOrigin::ExeCorporate),
            _ => None,
        }
    }
}

/// A registry-proven Windows installer command. Registry command strings are
/// never handed to a shell: MSI entries are reduced to a validated product GUID,
/// and Inno entries to a validated uninstaller executable inside InstallLocation.
#[cfg(windows)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum RegisteredUninstall {
    Msi { product_code: String },
    Inno { executable: PathBuf },
}

/// The registered installer that owns the running executable.
#[cfg(windows)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RegisteredInstallOwner {
    pub(crate) origin: InstallOrigin,
    pub(crate) install_location: PathBuf,
    pub(crate) uninstall: RegisteredUninstall,
    pub(crate) machine_hive: bool,
}

#[derive(Debug)]
enum StrategyError {
    /// Launcher / required tool not found on PATH or capability check failed.
    /// Safe to fall through — nothing was written.
    Preflight(String),
    /// The strategy ran end-to-end but reported a non-zero exit (or a verify
    /// mismatch). Cargo failures happen before origin cleanup; Unix archive
    /// replacement has an on-disk transaction marker and rollback.
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

/// Windows keeps an executing image locked against replacement, but it permits
/// an in-directory rename. A Cargo update writes back into the running binary's
/// own directory, so retire the two package binaries to versioned siblings
/// before invoking Cargo, then either delete them after success or restore them
/// if every strategy fails. MSI/Inno packages perform the equivalent operation
/// inside their elevated installer transaction; portable installs write to a
/// different Cargo directory and retain their existing scheduled-cleanup path.
#[cfg(windows)]
#[derive(Debug)]
struct RunningImageRetirement {
    files: Vec<(PathBuf, PathBuf)>,
}

#[cfg(windows)]
impl RunningImageRetirement {
    fn prepare(latest: &str) -> Result<Self, String> {
        let empty = || Self { files: Vec::new() };
        let Some(current) = current_exe_real_path() else {
            return Ok(empty());
        };
        let Some(dir) = current.parent() else {
            return Ok(empty());
        };
        let Some(cargo_bin) = cargo_bin_dir() else {
            return Ok(empty());
        };
        if !same_path(dir, &cargo_bin) {
            return Ok(empty());
        }
        let current_name = current
            .file_name()
            .map(|name| name.to_string_lossy().to_ascii_lowercase())
            .unwrap_or_default();
        if current_name != "nd300.exe" && current_name != "speedqx.exe" {
            return Ok(empty());
        }

        Self::prepare_in_dir(dir, latest)
    }

    fn prepare_in_dir(dir: &Path, latest: &str) -> Result<Self, String> {
        let mut files = Vec::new();
        for binary in super::uninstall::OUR_BINARIES {
            let original = dir.join(format!("{binary}.exe"));
            if !original.is_file() {
                continue;
            }
            let retired_name = retired_update_filename(binary, latest);
            if !crate::actions::migrate::is_retired_update_image(Path::new(&retired_name)) {
                return Err(format!(
                    "refusing unsafe retired-image version from release tag: {latest}"
                ));
            }
            let retired = dir.join(retired_name);
            if retired.exists() && std::fs::remove_file(&retired).is_err() {
                return abandon_retirement(
                    files,
                    format!("could not replace {}", retired.display()),
                );
            }
            if std::fs::rename(&original, &retired).is_err() {
                return abandon_retirement(
                    files,
                    format!("could not retire {}", original.display()),
                );
            }
            files.push((original, retired));
        }
        Ok(Self { files })
    }

    fn finish(self, success: bool) -> Result<(), String> {
        if success {
            for (_, retired) in self.files {
                if std::fs::remove_file(&retired).is_err() && retired.exists() {
                    if let Err(error) = super::uninstall::spawn_delayed_delete(&retired) {
                        eprintln!(
                            "  · WARNING: could not schedule retired update image {} for removal: {}",
                            retired.display(),
                            error
                        );
                    }
                }
            }
            Ok(())
        } else {
            restore_retired_images(&self.files)
        }
    }
}

#[cfg(windows)]
fn abandon_retirement(
    files: Vec<(PathBuf, PathBuf)>,
    reason: String,
) -> Result<RunningImageRetirement, String> {
    match restore_retired_images(&files) {
        Ok(()) => Err(format!("{reason}; original images restored")),
        Err(recovery) => Err(format!("{reason}; rollback also failed: {recovery}")),
    }
}

#[cfg(windows)]
fn restore_retired_images(files: &[(PathBuf, PathBuf)]) -> Result<(), String> {
    let mut failures = Vec::new();
    for (original, retired) in files.iter().rev() {
        if original.exists() {
            if let Err(error) = std::fs::remove_file(original) {
                failures.push(format!("could not remove {}: {error}", original.display()));
                continue;
            }
        }
        if retired.exists() {
            if let Err(error) = std::fs::rename(retired, original) {
                failures.push(format!(
                    "could not restore {} to {}: {error}",
                    retired.display(),
                    original.display()
                ));
            }
        } else if !original.exists() {
            failures.push(format!(
                "retired image {} disappeared before it could be restored",
                retired.display()
            ));
        }
    }
    if failures.is_empty() {
        Ok(())
    } else {
        Err(failures.join("; "))
    }
}

#[cfg(any(windows, test))]
fn retired_update_filename(binary: &str, version: &str) -> String {
    format!("{binary}.update-old-{version}.exe")
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

    let strategies = match build_strategy_list().await {
        Ok(strategies) => strategies,
        Err(error) => {
            println!(
                "  {} {}",
                color::red(fail_icon(config), config),
                color::red(&format!("Update stopped safely: {error}"), config)
            );
            println!("  Reinstall through the same method: {MANUAL_INSTALL_URL}");
            return 2;
        }
    };
    let primary = strategies.first().copied();
    if let Some(s) = primary {
        println!(
            "  {} Updating via {}...",
            color::cyan("*", config),
            s.label()
        );
    }
    println!();

    #[cfg(windows)]
    let retirement = match RunningImageRetirement::prepare(&latest) {
        Ok(retirement) => retirement,
        Err(error) => {
            println!(
                "  {} {}",
                color::red(fail_icon(config), config),
                color::red(&format!("Update preflight failed safely: {error}"), config)
            );
            println!("  Reinstall through the same method: {MANUAL_INSTALL_URL}");
            return 2;
        }
    };
    let mut result = execute_update(&latest, &strategies).await;
    #[cfg(windows)]
    if let Err(error) = retirement.finish(result.is_ok()) {
        if let Err(failure) = &mut result {
            failure.attempts.push(AttemptRecord {
                strategy: strategies
                    .first()
                    .copied()
                    .unwrap_or(UpdateStrategy::InstallerPowerShell),
                kind: AttemptKind::Failed,
                message: format!("running-image rollback failed: {error}"),
            });
        }
    }

    match result {
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
            println!("  Reinstall through the same method: {MANUAL_INSTALL_URL}");
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

    let strategies = match build_strategy_list().await {
        Ok(strategies) => strategies,
        Err(error) => {
            let mut output = serde_json::json!({
                "action": "update",
                "success": false,
                "message": format!("Update stopped safely: {error}"),
                "current_version": current,
                "latest_version": latest,
                "update_available": true,
                "attempts": [],
                "manual_install_url": MANUAL_INSTALL_URL,
            });
            inject_install_origin(&mut output);
            println!(
                "{}",
                serde_json::to_string_pretty(&output).unwrap_or_else(|_| "{}".to_string())
            );
            return 2;
        }
    };
    #[cfg(windows)]
    let retirement = match RunningImageRetirement::prepare(&latest) {
        Ok(retirement) => retirement,
        Err(error) => {
            let mut output = serde_json::json!({
                "action": "update",
                "success": false,
                "message": format!("Update preflight failed safely: {error}"),
                "current_version": current,
                "latest_version": latest,
                "update_available": true,
                "attempts": [],
                "manual_install_url": MANUAL_INSTALL_URL,
            });
            inject_install_origin(&mut output);
            println!(
                "{}",
                serde_json::to_string_pretty(&output).unwrap_or_else(|_| "{}".to_string())
            );
            return 2;
        }
    };
    let mut result = execute_update(&latest, &strategies).await;
    #[cfg(windows)]
    if let Err(error) = retirement.finish(result.is_ok()) {
        if let Err(failure) = &mut result {
            failure.attempts.push(AttemptRecord {
                strategy: strategies
                    .first()
                    .copied()
                    .unwrap_or(UpdateStrategy::InstallerPowerShell),
                kind: AttemptKind::Failed,
                message: format!("running-image rollback failed: {error}"),
            });
        }
    }

    match result {
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
                "manual_install_url": MANUAL_INSTALL_URL,
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

    // `/releases/latest` excludes prereleases. Validate the stable numeric tag
    // before interpolating it into immutable artifact URLs or Cargo arguments.
    let version = tag.strip_prefix('v').unwrap_or(tag);
    validate_release_version(version)?;
    Ok(version.to_string())
}

fn validate_release_version(version: &str) -> Result<(), String> {
    if version.len() > 32 {
        return Err("release version is unreasonably long".to_string());
    }
    let mut parts = version.split('.');
    let valid = (0..3).all(|_| {
        parts
            .next()
            .is_some_and(|part| !part.is_empty() && part.chars().all(|c| c.is_ascii_digit()))
    }) && parts.next().is_none();
    if valid {
        Ok(())
    } else {
        Err(format!("invalid stable release version {version:?}"))
    }
}

fn exact_release_asset_url(version: &str, asset: &str) -> Result<String, StrategyError> {
    validate_release_version(version).map_err(StrategyError::Preflight)?;
    Ok(format!("{RELEASE_DOWNLOAD_BASE}/v{version}/{asset}"))
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

// ─── Origin-preserving strategy planning ─────────────────────────────────────

#[cfg(any(windows, test))]
fn windows_cargo_or_installer_plan(
    cargo_registered: bool,
    cargo_invokable: bool,
) -> Result<Vec<UpdateStrategy>, String> {
    if cargo_registered {
        return cargo_invokable
            .then_some(vec![UpdateStrategy::Cargo])
            .ok_or_else(|| {
                "this copy is registered to Cargo, but the owning Cargo executable is unavailable"
                    .to_string()
            });
    }
    Ok(vec![
        UpdateStrategy::InstallerPowerShell,
        UpdateStrategy::InstallerPwsh,
    ])
}

#[cfg(windows)]
async fn cargo_invokable() -> bool {
    tool_exists("cargo")
}

#[cfg(windows)]
fn tool_exists(tool: &str) -> bool {
    Command::new(tool)
        .arg("--version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

async fn build_strategy_list() -> Result<Vec<UpdateStrategy>, String> {
    #[cfg(windows)]
    {
        // For first-class Windows installs, dispatch to a single matching
        // installer strategy. Don't cross-fall-back to a different installer type
        // — running a different product would create coexistence problems (two
        // ARP entries, PATH ordering wins).
        match detect_install_origin() {
            InstallOrigin::MsiGlobal => Ok(vec![UpdateStrategy::MsiGlobal]),
            InstallOrigin::MsiCorporate => Ok(vec![UpdateStrategy::MsiCorporate]),
            InstallOrigin::ExeGlobal => Ok(vec![UpdateStrategy::ExeGlobal]),
            InstallOrigin::ExeCorporate => Ok(vec![UpdateStrategy::ExeCorporate]),
            InstallOrigin::CargoOrInstaller => windows_cargo_or_installer_plan(
                windows_cargo_package_registered(),
                cargo_invokable().await,
            ),
            InstallOrigin::Unknown => Err(
                "the running executable is not owned by a supported installer; no files were changed"
                    .to_string(),
            ),
        }
    }

    #[cfg(unix)]
    {
        use super::unix_install::UnixOriginKind;

        let user = crate::platform::invoking_user::InvokingUser::detect()
            .map_err(|error| format!("could not identify the invoking user: {error}"))?;
        let origin = super::unix_install::detect_origin(&user)
            .map_err(|error| format!("could not identify the install owner: {error}"))?;
        let kind = super::unix_install::effective_update_kind_for(&origin, &user);
        match kind {
            UnixOriginKind::Cargo => {
                if super::unix_install::cargo_is_invokable(&user).await {
                    Ok(vec![UpdateStrategy::Cargo])
                } else {
                    Err(
                        "this copy is registered to Cargo, but the owning Cargo executable is unavailable"
                            .to_string(),
                    )
                }
            }
            UnixOriginKind::ManagedArchive => Ok(vec![UpdateStrategy::UnixArchive]),
            UnixOriginKind::PackageManager => Err(format!(
                "the running executable at {} is package-manager-owned; update it through that package manager",
                origin.executable.display()
            )),
            UnixOriginKind::LocalBuild => Err(format!(
                "the running executable at {} is a local build; rebuild it from source",
                origin.executable.display()
            )),
            UnixOriginKind::Symlink | UnixOriginKind::Unknown => Err(format!(
                "the install owner of {} could not be proven; no files were changed",
                origin.executable.display()
            )),
        }
    }
}

// ─── Strategy execution ──────────────────────────────────────────────────────

async fn execute_update(
    latest: &str,
    strategies: &[UpdateStrategy],
) -> Result<UpdateStrategy, UpdateFailure> {
    let mut attempts = Vec::new();
    for &strategy in strategies {
        match try_strategy(strategy, latest).await {
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

async fn try_strategy(strategy: UpdateStrategy, latest: &str) -> Result<(), StrategyError> {
    match strategy {
        UpdateStrategy::Cargo => try_cargo_install(latest).await,
        #[cfg(unix)]
        UpdateStrategy::UnixArchive => try_unix_archive(latest).await,
        UpdateStrategy::InstallerPowerShell => try_installer_powershell("powershell", latest),
        UpdateStrategy::InstallerPwsh => try_installer_powershell("pwsh", latest),
        UpdateStrategy::MsiGlobal => {
            try_msi_install(&exact_release_asset_url(latest, MSI_GLOBAL_ASSET)?, latest)
        }
        UpdateStrategy::MsiCorporate => try_msi_install(
            &exact_release_asset_url(latest, MSI_CORPORATE_ASSET)?,
            latest,
        ),
        UpdateStrategy::ExeGlobal => {
            try_exe_install(&exact_release_asset_url(latest, EXE_GLOBAL_ASSET)?, latest)
        }
        UpdateStrategy::ExeCorporate => try_exe_install(
            &exact_release_asset_url(latest, EXE_CORPORATE_ASSET)?,
            latest,
        ),
    }
}

async fn try_cargo_install(latest: &str) -> Result<(), StrategyError> {
    #[cfg(unix)]
    {
        let user = crate::platform::invoking_user::InvokingUser::detect()
            .map_err(|error| StrategyError::Preflight(format!("invoking user: {error}")))?;
        let preinstall_origin = super::unix_install::detect_origin(&user)
            .map_err(|error| StrategyError::Preflight(format!("install origin: {error}")))?;
        super::unix_install::cargo_install(latest, &user)
            .await
            .map_err(StrategyError::Runtime)?;
        cleanup_shadowing_current_install_after_cargo_success(&preinstall_origin, &user)?;
        return Ok(());
    }

    #[cfg(windows)]
    {
        let args = [
            "install",
            "nd300",
            "--version",
            latest,
            "--force",
            "--locked",
        ];
        match run_command_capture("cargo", &args) {
            Ok(()) => {
                // Clean up any shadowing non-cargo install first (ND-300's existing
                // machinery), THEN confirm the running binary actually changed.
                // Cargo exit 0 doesn't guarantee that the canonical pair now
                // reports the exact pinned release, so verify before committing
                // the running-image retirement.
                cleanup_shadowing_current_install_after_cargo_success()?;
                verify_cargo_post_install(latest)
            }
            Err(StrategyError::Runtime(msg))
                if cargo_failure_suggests_existing_binary_collision(&msg) =>
            {
                cleanup_current_install_for_cargo_retry()?;
                run_command_capture("cargo", &args)?;
                verify_cargo_post_install(latest)
            }
            other => other,
        }
    }
}

#[cfg(unix)]
async fn try_unix_archive(latest: &str) -> Result<(), StrategyError> {
    let user = crate::platform::invoking_user::InvokingUser::detect()
        .map_err(|error| StrategyError::Preflight(format!("invoking user: {error}")))?;
    super::unix_install::update_from_archive(latest, &user)
        .await
        .map_err(StrategyError::Fatal)
}

/// The three outcomes of a shadow-cleanup attempt, as a function of the cleanup
/// report. Pure so it can be unit-tested without touching the filesystem.
/// Shared with `migrate-cleanup` (src/actions/migrate.rs) so both the updater and
/// the installer-driven consolidation map a `CleanupReport` the same way.
#[derive(Debug, PartialEq, Eq)]
#[cfg_attr(not(windows), allow(dead_code))]
pub(crate) enum ShadowCleanupDecision {
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
#[cfg_attr(not(windows), allow(dead_code))]
pub(crate) fn classify_shadow_cleanup(
    report: &super::uninstall::CleanupReport,
) -> ShadowCleanupDecision {
    if report.binary_removed {
        ShadowCleanupDecision::Removed
    } else if report.binary_removal_scheduled {
        ShadowCleanupDecision::Scheduled
    } else {
        ShadowCleanupDecision::NotRemoved
    }
}

#[cfg(windows)]
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

#[cfg(unix)]
fn cleanup_shadowing_current_install_after_cargo_success(
    origin: &super::unix_install::UnixInstallOrigin,
    user: &crate::platform::invoking_user::InvokingUser,
) -> Result<(), StrategyError> {
    let cargo_bin = user.cargo_home().join("bin");
    if origin
        .executable
        .parent()
        .is_some_and(|parent| same_path(parent, &cargo_bin))
    {
        return Ok(());
    }
    Err(StrategyError::Fatal(format!(
        "cargo installed and verified the requested release in {}, but the running copy at {} has an unknown or externally managed origin and was not deleted. Start a new shell with {} first on PATH, then remove the old copy through its original installer.",
        cargo_bin.display(),
        origin.executable.display(),
        cargo_bin.display()
    )))
}

#[cfg(windows)]
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

#[cfg(windows)]
fn cleanup_notes(report: &super::uninstall::CleanupReport) -> String {
    if report.notes.is_empty() {
        "no additional details".to_string()
    } else {
        report.notes.join("; ")
    }
}

#[cfg_attr(not(windows), allow(dead_code))]
pub(crate) fn current_exe_real_path() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    Some(exe.canonicalize().unwrap_or(exe))
}

#[cfg_attr(not(windows), allow(dead_code))]
pub(crate) fn cargo_bin_dir() -> Option<PathBuf> {
    if let Some(home) = std::env::var_os("CARGO_HOME") {
        return Some(PathBuf::from(home).join("bin"));
    }

    #[cfg(windows)]
    {
        std::env::var_os("USERPROFILE").map(|home| PathBuf::from(home).join(".cargo").join("bin"))
    }

    #[cfg(unix)]
    {
        crate::platform::invoking_user::InvokingUser::detect()
            .ok()
            .map(|user| user.cargo_home().join("bin"))
    }
}

#[cfg(windows)]
fn windows_cargo_package_registered() -> bool {
    cargo_bin_dir()
        .and_then(|bin| bin.parent().map(Path::to_path_buf))
        .is_some_and(|home| windows_cargo_package_registered_at(&home))
}

#[cfg(any(windows, test))]
fn windows_cargo_package_registered_at(cargo_home: &Path) -> bool {
    const LIMIT: u64 = 2 * 1024 * 1024;
    let read_regular = |path: &Path| -> Option<Vec<u8>> {
        let metadata = std::fs::symlink_metadata(path).ok()?;
        if !metadata.file_type().is_file() || metadata.len() > LIMIT {
            return None;
        }
        std::fs::read(path)
            .ok()
            .filter(|bytes| bytes.len() as u64 <= LIMIT)
    };

    if let Some(bytes) = read_regular(&cargo_home.join(".crates2.json")) {
        if serde_json::from_slice::<serde_json::Value>(&bytes)
            .ok()
            .and_then(|value| {
                value
                    .get("installs")
                    .and_then(serde_json::Value::as_object)
                    .cloned()
            })
            .is_some_and(|installs| installs.keys().any(|key| cargo_registry_key_is_nd300(key)))
        {
            return true;
        }
    }

    read_regular(&cargo_home.join(".crates.toml"))
        .and_then(|bytes| String::from_utf8(bytes).ok())
        .is_some_and(|text| {
            text.lines()
                .any(|line| cargo_registry_key_is_nd300(line.trim()))
        })
}

#[cfg(any(windows, test))]
fn cargo_registry_key_is_nd300(key: &str) -> bool {
    key.trim_start_matches(['"', '\''])
        .strip_prefix("nd300 ")
        .is_some_and(|rest| rest.chars().next().is_some_and(|c| c.is_ascii_digit()))
}

#[cfg_attr(not(windows), allow(dead_code))]
pub(crate) fn current_install_shadows_cargo_install(
    current_exe: &Path,
    cargo_bin_dir: &Path,
) -> bool {
    if current_exe_looks_like_local_build(current_exe) {
        return false;
    }

    let Some(current_dir) = current_exe.parent() else {
        return false;
    };
    !same_path(current_dir, cargo_bin_dir)
}

#[cfg_attr(not(windows), allow(dead_code))]
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

pub(crate) fn same_path(left: &Path, right: &Path) -> bool {
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

#[cfg(any(windows, test))]
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
#[cfg(windows)]
fn run_command_status(launcher: &Path, args: &[&str]) -> Result<(), StrategyError> {
    let res = Command::new(launcher).args(args).status();
    match res {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Err(StrategyError::Preflight(
            format!("{} was not found", launcher.display()),
        )),
        Err(e) => Err(StrategyError::Preflight(format!(
            "Failed to spawn {}: {}",
            launcher.display(),
            e
        ))),
        Ok(s) if s.success() => Ok(()),
        Ok(s) => Err(StrategyError::Runtime(format!(
            "{} exited with code {}",
            launcher.display(),
            s.code().unwrap_or(-1)
        ))),
    }
}

#[cfg(windows)]
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

#[cfg(windows)]
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

#[cfg(windows)]
fn try_installer_powershell(launcher: &str, latest: &str) -> Result<(), StrategyError> {
    // `$ErrorActionPreference='Stop'` so an `irm` failure aborts the pipeline
    // rather than feeding nothing into iex (which would exit 0 and look like a
    // successful update).
    let installer = exact_release_asset_url(latest, PS_INSTALLER_ASSET)?;
    let script = format!("$ErrorActionPreference='Stop'; irm {installer} | iex");
    let launcher = if launcher.eq_ignore_ascii_case("powershell") {
        super::uninstall::system_powershell_path().map_err(StrategyError::Preflight)?
    } else {
        PathBuf::from(launcher)
    };
    run_command_status(
        &launcher,
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
fn try_installer_powershell(_launcher: &str, _latest: &str) -> Result<(), StrategyError> {
    Err(StrategyError::Preflight(
        "PowerShell installer is Windows-only".into(),
    ))
}

// ─── Windows MSI / EXE installer strategies (v3.1.0+) ─────────────────────────

/// msiexec exit code returned when the install completed successfully but a
/// reboot is required to finalize a file replacement. The ND-300 MSI normally
/// retires both running images through its transactional MoveFiles actions, but
/// this remains a valid Windows Installer success code that needs disclosure.
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
/// corruption detection before execution. Authenticity is enforced separately
/// by the signed installer/release trust chain.
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
/// The same-origin sidecar detects truncation and accidental/cached corruption.
/// It is not an authenticity proof against an attacker who can replace both the
/// installer and sidecar; platform signing and release attestations provide that
/// separate trust layer.
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
            "SHA256 mismatch — refusing to run installer.\n         Expected: {}\n         Got:      {}\n         The download may be corrupted or tampered with.",
            expected, actual
        ))
    }
}

/// Whether the freshly-installed `--version` string matches the expected release
/// tag. Both sides are stripped of any prerelease/build-metadata suffix before
/// comparison, and an empty `installed` never matches (covers `--version`
/// failing to parse). Pure + platform-independent.
#[cfg(any(windows, test))]
fn post_install_version_ok(installed: &str, expected: &str) -> bool {
    let installed_stripped = strip_prerelease_metadata(installed);
    let expected_stripped = strip_prerelease_metadata(expected);
    !installed_stripped.is_empty() && installed_stripped == expected_stripped
}

/// Resolve a process image that has been retired for an in-place update back to
/// its canonical package filename. `GetModuleFileName` normally preserves the
/// name used to launch the process, but this makes post-install verification
/// independent of that Windows implementation detail.
#[cfg(any(windows, test))]
fn canonical_update_executable(exe: &Path) -> PathBuf {
    let name = exe
        .file_name()
        .map(|value| value.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default();
    let canonical = if name.starts_with("nd300.update-old-") && name.ends_with(".exe") {
        Some("nd300.exe")
    } else if name.starts_with("speedqx.update-old-") && name.ends_with(".exe") {
        Some("speedqx.exe")
    } else {
        None
    };
    canonical
        .and_then(|filename| exe.parent().map(|dir| dir.join(filename)))
        .unwrap_or_else(|| exe.to_path_buf())
}

/// Re-exec the newly installed canonical binary with `--version` and return the
/// parsed version (the last whitespace token of `nd300 X.Y.Z` / `speedqx X.Y.Z`).
/// `None` if the spawn fails, the process errors, or the output doesn't parse.
#[cfg(windows)]
fn reexec_installed_version() -> Option<String> {
    let exe = canonical_update_executable(&std::env::current_exe().ok()?);
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

/// Confirm an exact-version `cargo install nd300` update actually landed.
///
/// `cargo install` can report success (exit 0) while a shadowing or incomplete
/// replacement still leaves the canonical command at the old version. Re-exec
/// `--version`; a mismatch fails this Cargo-only route and restores the retired
/// images rather than crossing to a different installation channel.
#[cfg(windows)]
fn verify_cargo_post_install(expected: &str) -> Result<(), StrategyError> {
    match reexec_installed_version() {
        Some(installed) if post_install_version_ok(&installed, expected) => Ok(()),
        Some(installed) => Err(StrategyError::Runtime(format!(
            "cargo install reported success but `nd300 --version` still reports v{installed} (expected v{expected}); the Cargo update was not committed and no other installation channel was attempted."
        ))),
        None => Err(StrategyError::Runtime(
            "cargo install reported success but `nd300 --version` could not be run to confirm the new version; the Cargo update was not committed and no other installation channel was attempted.".to_string(),
        )),
    }
}

/// After the installer reports success, re-exec `<current_exe> --version` and
/// confirm the on-disk binary has been updated. Catches the case where the
/// installer exits 0 but the file replacement didn't take effect (a deferred
/// replacement, MSI re-running the same version, etc.).
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
/// uninstall-old-then-install-new step atomically. The packages disable Restart
/// Manager and use transactional MoveFiles actions to retire the locked pair
/// before replacement. If Windows Installer still returns 3010, we surface it
/// without claiming a fully finalized update.
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
        let msiexec = super::uninstall::system_msiexec_path().map_err(StrategyError::Preflight)?;
        let status = Command::new(&msiexec)
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
/// four first-class installers on install. The marker is a hint, not proof of
/// ownership: registered uninstall metadata and a running `.cargo\bin` path take
/// precedence over it.
#[cfg(windows)]
pub(crate) fn read_install_source_marker() -> Option<InstallOrigin> {
    use winreg::enums::HKEY_CURRENT_USER;
    use winreg::RegKey;

    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    let key = hkcu.open_subkey("Software\\ND300").ok()?;
    let value: String = key.get_value("InstallSource").ok()?;
    InstallOrigin::from_marker(value.trim())
}

/// Determine how this binary was installed on Windows.
///
/// Ownership precedence is deliberate: a `.cargo\bin` executable always wins;
/// then a unique HKCU/HKLM Add/Remove Programs record whose normalized
/// InstallLocation owns the executable; then a marker compatible with the
/// path's edition; finally the path-only fallback. This prevents a stale marker
/// from steering a surviving Cargo copy to an unrelated installer.
#[cfg(windows)]
fn detect_install_origin() -> InstallOrigin {
    let Ok(exe) = std::env::current_exe() else {
        return InstallOrigin::Unknown;
    };
    detect_install_origin_for_path(&exe)
}

#[cfg(windows)]
pub(crate) fn detect_install_origin_for_path(exe: &Path) -> InstallOrigin {
    let path_origin = classify_install_path(&exe.to_string_lossy());

    // A Cargo install can legitimately survive an installer uninstall. The
    // shared HKCU marker may still be stale for a few moments (or may have been
    // left by an older release), so never let it override this unambiguous path.
    if path_origin == InstallOrigin::CargoOrInstaller {
        return path_origin;
    }

    let owner_origin = registered_install_owner(exe).map(|owner| owner.origin);
    resolve_install_origin(path_origin, read_install_source_marker(), owner_origin)
}

#[cfg(windows)]
fn resolve_install_origin(
    path_origin: InstallOrigin,
    marker: Option<InstallOrigin>,
    registered_owner: Option<InstallOrigin>,
) -> InstallOrigin {
    if path_origin == InstallOrigin::CargoOrInstaller {
        return path_origin;
    }
    if let Some(owner) = registered_owner {
        return owner;
    }

    match (path_origin, marker) {
        (
            InstallOrigin::MsiGlobal,
            Some(marker @ (InstallOrigin::MsiGlobal | InstallOrigin::ExeGlobal)),
        ) => marker,
        (
            InstallOrigin::MsiCorporate,
            Some(marker @ (InstallOrigin::MsiCorporate | InstallOrigin::ExeCorporate)),
        ) => marker,
        _ => path_origin,
    }
}

/// Pure-function half of `detect_install_origin()` for unit testing. Lowercased
/// substring match handles drive-letter casing and Windows path
/// case-insensitivity. Order matters: check more-specific paths first.
///
/// LOCKSTEP: these substrings must match the install dirs in wix/main.wxs
/// (Program Files\nd300, plus the pre-v3.1 Program Files\nd-300 location),
/// wix-corporate/corporate.wxs + inno/corporate.iss
/// (LocalAppData\Programs\nd300), and the cargo bin (`.cargo\bin`).
#[cfg(windows)]
pub(crate) fn classify_install_path(exe_path: &str) -> InstallOrigin {
    let lower = exe_path.to_lowercase();
    if lower.contains("\\.cargo\\bin\\") {
        InstallOrigin::CargoOrInstaller
    } else if lower.contains("\\program files\\nd300\\")
        || lower.contains("\\program files\\nd-300\\")
    {
        InstallOrigin::MsiGlobal
    } else if lower.contains("\\appdata\\local\\programs\\nd300\\") {
        InstallOrigin::MsiCorporate
    } else {
        InstallOrigin::Unknown
    }
}

/// Resolve the registered MSI/Inno owner of `exe_path`, if exactly one proven
/// Add/Remove Programs record owns its normalized install directory. If both
/// formats are genuinely registered to the same directory, the marker may
/// disambiguate only between those proven candidates; otherwise we refuse to
/// guess.
#[cfg(windows)]
pub(crate) fn registered_install_owner(exe_path: &Path) -> Option<RegisteredInstallOwner> {
    let mut candidates: Vec<_> = registered_install_owners()
        .into_iter()
        .filter(|owner| install_location_owns_executable(&owner.install_location, exe_path))
        .collect();

    if candidates.len() == 1 {
        return candidates.pop();
    }
    let marker = read_install_source_marker()?;
    let mut matching = candidates
        .into_iter()
        .filter(|candidate| candidate.origin == marker);
    let selected = matching.next()?;
    if matching.next().is_none() {
        Some(selected)
    } else {
        None
    }
}

/// Enumerate every registry-proven ND-300 installer owner. This is broader than
/// `registered_install_owner`: a deliberate fresh installer uses it to remove
/// conflicting channels before claiming ownership. Exact display names,
/// installer identities, product codes/uninstaller paths, and an on-disk ND-300
/// binary under InstallLocation are all required.
#[cfg(windows)]
pub(crate) fn registered_install_owners() -> Vec<RegisteredInstallOwner> {
    registered_install_owners_impl(false).unwrap_or_default()
}

/// Strict owner inventory for fresh-installer takeover. Unlike ordinary origin
/// classification, this path may execute uninstallers, so registry enumeration
/// errors fail closed and MSI product codes must be enumerated from the
/// canonical UpgradeCode by Windows Installer itself.
#[cfg(windows)]
pub(crate) fn registered_install_owners_for_takeover() -> Result<Vec<RegisteredInstallOwner>, String>
{
    let owners = registered_install_owners_impl(true)?;
    let global_products = msi_related_product_codes("{7208E6EA-ABAE-4AF6-A69C-7C7E4DA3DDF5}")?;
    let corporate_products = msi_related_product_codes("{B2490B7E-3CBE-47AC-8090-79BB3EB555E0}")?;

    for owner in &owners {
        let RegisteredUninstall::Msi { product_code } = &owner.uninstall else {
            continue;
        };
        let related = match owner.origin {
            InstallOrigin::MsiGlobal => &global_products,
            InstallOrigin::MsiCorporate => &corporate_products,
            _ => {
                return Err(format!(
                    "internal installer identity mismatch for {}",
                    owner.origin.json_id()
                ))
            }
        };
        if !related.contains(product_code) {
            return Err(format!(
                "refusing unproven MSI product {} for {}",
                product_code,
                owner.origin.json_id()
            ));
        }
    }
    Ok(owners)
}

#[cfg(windows)]
fn registered_install_owners_impl(
    strict_enumeration: bool,
) -> Result<Vec<RegisteredInstallOwner>, String> {
    use winreg::enums::{
        HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, KEY_READ, KEY_WOW64_32KEY, KEY_WOW64_64KEY,
    };
    use winreg::RegKey;

    const UNINSTALL_KEY: &str = "Software\\Microsoft\\Windows\\CurrentVersion\\Uninstall";
    let roots = [
        (RegKey::predef(HKEY_CURRENT_USER), false),
        (RegKey::predef(HKEY_LOCAL_MACHINE), true),
    ];
    let views = [KEY_READ | KEY_WOW64_64KEY, KEY_READ | KEY_WOW64_32KEY];
    let mut candidates = Vec::new();

    for (root, machine_hive) in roots {
        for flags in views {
            let uninstall_root = match root.open_subkey_with_flags(UNINSTALL_KEY, flags) {
                Ok(key) => key,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
                Err(_) if !strict_enumeration => continue,
                Err(error) => {
                    return Err(format!(
                        "could not enumerate a Windows uninstall registry view: {error}"
                    ))
                }
            };
            for key_name in uninstall_root.enum_keys() {
                let key_name = match key_name {
                    Ok(name) => name,
                    Err(_) if !strict_enumeration => continue,
                    Err(error) => {
                        return Err(format!(
                            "could not enumerate a Windows uninstall registry key: {error}"
                        ))
                    }
                };
                let key = match uninstall_root.open_subkey_with_flags(&key_name, flags) {
                    Ok(key) => key,
                    Err(_) if !strict_enumeration => continue,
                    Err(error) => {
                        return Err(format!(
                            "could not inspect Windows uninstall key {key_name}: {error}"
                        ))
                    }
                };
                let Ok(display_name) = key.get_value::<String, _>("DisplayName") else {
                    continue;
                };
                let Some(msi_origin) =
                    msi_origin_for_registered_record(&display_name, machine_hive)
                else {
                    continue;
                };

                let Ok(location_text) = key.get_value::<String, _>("InstallLocation") else {
                    continue;
                };
                let install_location = PathBuf::from(location_text.trim());
                if install_location.as_os_str().is_empty()
                    || !nd300_binary_exists(&install_location)
                {
                    continue;
                }

                let owner = if let Some(origin) = inno_origin_for_key(&key_name) {
                    let expected_msi_origin = match origin {
                        InstallOrigin::ExeGlobal => InstallOrigin::MsiGlobal,
                        InstallOrigin::ExeCorporate => InstallOrigin::MsiCorporate,
                        _ => continue,
                    };
                    if msi_origin != expected_msi_origin {
                        continue;
                    }
                    let Ok(raw_uninstall) = key.get_value::<String, _>("UninstallString") else {
                        continue;
                    };
                    let Some(executable) =
                        validated_inno_uninstaller(&raw_uninstall, &install_location)
                    else {
                        continue;
                    };
                    if !executable.is_file() {
                        continue;
                    }
                    RegisteredInstallOwner {
                        origin,
                        install_location,
                        uninstall: RegisteredUninstall::Inno { executable },
                        machine_hive,
                    }
                } else {
                    let windows_installer = key
                        .get_value::<u32, _>("WindowsInstaller")
                        .unwrap_or_default()
                        == 1;
                    let Some(product_code) = normalize_product_code(&key_name) else {
                        continue;
                    };
                    if !windows_installer {
                        continue;
                    }
                    RegisteredInstallOwner {
                        origin: msi_origin,
                        install_location,
                        uninstall: RegisteredUninstall::Msi { product_code },
                        machine_hive,
                    }
                };

                if !candidates.contains(&owner) {
                    candidates.push(owner);
                }
            }
        }
    }
    Ok(candidates)
}

#[cfg(windows)]
fn msi_related_product_codes(
    upgrade_code: &str,
) -> Result<std::collections::HashSet<String>, String> {
    use std::os::windows::ffi::{OsStrExt, OsStringExt};

    #[link(name = "msi")]
    unsafe extern "system" {
        fn MsiEnumRelatedProductsW(
            upgrade_code: *const u16,
            reserved: u32,
            product_index: u32,
            product_code: *mut u16,
        ) -> u32;
    }

    const ERROR_SUCCESS: u32 = 0;
    const ERROR_NO_MORE_ITEMS: u32 = 259;
    const PRODUCT_CODE_CHARS: usize = 39;
    const ENUMERATION_LIMIT: u32 = 1024;

    let wide: Vec<u16> = std::ffi::OsStr::new(upgrade_code)
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    let mut products = std::collections::HashSet::new();
    for index in 0..ENUMERATION_LIMIT {
        let mut buffer = [0_u16; PRODUCT_CODE_CHARS];
        // SAFETY: `wide` is NUL-terminated and `buffer` is the documented
        // 39-WCHAR product-code output buffer for the duration of the call.
        let result =
            unsafe { MsiEnumRelatedProductsW(wide.as_ptr(), 0, index, buffer.as_mut_ptr()) };
        match result {
            ERROR_SUCCESS => {
                let length = buffer.iter().position(|value| *value == 0).unwrap_or(buffer.len());
                let raw = std::ffi::OsString::from_wide(&buffer[..length])
                    .to_string_lossy()
                    .into_owned();
                let normalized = normalize_product_code(&raw).ok_or_else(|| {
                    format!("Windows Installer returned an invalid product code {raw:?}")
                })?;
                products.insert(normalized);
            }
            ERROR_NO_MORE_ITEMS => return Ok(products),
            error => {
                return Err(format!(
                    "Windows Installer could not enumerate products related to {upgrade_code}: error {error}"
                ))
            }
        }
    }
    Err(format!(
        "Windows Installer returned too many products related to {upgrade_code}"
    ))
}

#[cfg(windows)]
fn nd300_binary_exists(install_location: &Path) -> bool {
    [
        install_location.join("nd300.exe"),
        install_location.join("bin").join("nd300.exe"),
    ]
    .into_iter()
    .any(|candidate| candidate.is_file())
}

/// Determine the MSI edition represented by an exact ARP display name and
/// registry hive. Global ownership is accepted only from protected HKLM.
/// Corporate ownership normally lives in HKCU, but Windows Installer may expose
/// a per-user-managed/elevated registration from protected HKLM; both are safe
/// because the exact Corporate identity and owned install location are still
/// required. A user-writable HKCU record can never claim the Global edition.
#[cfg(windows)]
fn msi_origin_for_registered_record(
    display_name: &str,
    machine_hive: bool,
) -> Option<InstallOrigin> {
    let display_name = display_name.trim();
    if machine_hive
        && (display_name.eq_ignore_ascii_case("nd300")
            || display_name.eq_ignore_ascii_case("nd-300"))
    {
        Some(InstallOrigin::MsiGlobal)
    } else if display_name.eq_ignore_ascii_case("nd300 (Corporate Edition)") {
        Some(InstallOrigin::MsiCorporate)
    } else {
        None
    }
}

#[cfg(windows)]
fn install_location_owns_executable(install_location: &Path, exe_path: &Path) -> bool {
    let Some(file_name) = exe_path.file_name().and_then(|name| name.to_str()) else {
        return false;
    };
    if !matches!(
        file_name.to_ascii_lowercase().as_str(),
        "nd300.exe" | "speedqx.exe"
    ) {
        return false;
    }
    let Some(exe_dir) = exe_path.parent() else {
        return false;
    };
    same_path(exe_dir, install_location) || same_path(exe_dir, &install_location.join("bin"))
}

#[cfg(windows)]
fn inno_origin_for_key(key_name: &str) -> Option<InstallOrigin> {
    const GLOBAL: &str = "{13F102E1-E08D-4C4E-ABA6-7D77604DFECD}_IS1";
    const CORPORATE: &str = "{B6A0E3BD-BDD8-44A3-B524-C226B2A116A9}_IS1";
    let upper = key_name.trim().to_ascii_uppercase();
    match upper.as_str() {
        GLOBAL => Some(InstallOrigin::ExeGlobal),
        CORPORATE => Some(InstallOrigin::ExeCorporate),
        _ => None,
    }
}

#[cfg(windows)]
fn validated_inno_uninstaller(raw: &str, install_location: &Path) -> Option<PathBuf> {
    let trimmed = raw.trim();
    let path_text = if let Some(quoted) = trimmed.strip_prefix('"') {
        &quoted[..quoted.find('"')?]
    } else {
        trimmed.split_whitespace().next()?
    };
    let executable = PathBuf::from(path_text);
    let file_name = executable
        .file_name()?
        .to_string_lossy()
        .to_ascii_lowercase();
    if !file_name.starts_with("unins") || !file_name.ends_with(".exe") {
        return None;
    }
    if !same_path(executable.parent()?, install_location) {
        return None;
    }
    Some(executable)
}

#[cfg(windows)]
fn normalize_product_code(raw: &str) -> Option<String> {
    let upper = raw.trim().to_ascii_uppercase();
    let bytes = upper.as_bytes();
    if bytes.len() != 38 || bytes[0] != b'{' || bytes[37] != b'}' {
        return None;
    }
    for (index, byte) in bytes.iter().enumerate().take(37).skip(1) {
        if matches!(index, 9 | 14 | 19 | 24) {
            if *byte != b'-' {
                return None;
            }
        } else if !byte.is_ascii_hexdigit() {
            return None;
        }
    }
    Some(upper)
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retired_update_names_match_the_migration_allowlist() {
        for binary in ["nd300", "speedqx"] {
            let name = retired_update_filename(binary, "3.6.3");
            assert!(crate::actions::migrate::is_retired_update_image(Path::new(
                &name
            )));
        }
    }

    #[test]
    fn post_install_reexec_uses_canonical_name_after_retirement() {
        let dir = PathBuf::from("install-bin");
        assert_eq!(
            canonical_update_executable(&dir.join("nd300.update-old-3.6.3.exe")),
            dir.join("nd300.exe")
        );
        assert_eq!(
            canonical_update_executable(&dir.join("SPEEDQX.UPDATE-OLD-3.6.3-RC.1.EXE")),
            dir.join("speedqx.exe")
        );
        assert_eq!(
            canonical_update_executable(&dir.join("nd300.exe")),
            dir.join("nd300.exe")
        );
        assert_eq!(
            canonical_update_executable(&dir.join("unrelated.update-old-3.6.3.exe")),
            dir.join("unrelated.update-old-3.6.3.exe")
        );
    }

    #[cfg(windows)]
    #[test]
    fn running_cargo_image_can_be_retired_and_restored_without_termination() {
        use std::process::{Child, Command, Stdio};
        use std::time::{SystemTime, UNIX_EPOCH};

        struct ChildGuard(Child);
        impl Drop for ChildGuard {
            fn drop(&mut self) {
                let _ = self.0.kill();
                let _ = self.0.wait();
            }
        }

        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock before Unix epoch")
            .as_nanos();
        let dir = std::env::temp_dir().join(format!(
            "nd300-running-image-{}-{unique}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).expect("create running-image fixture directory");

        let system_root = std::env::var_os("SystemRoot").expect("SystemRoot is set");
        let system32 = PathBuf::from(system_root).join("System32");
        let old_image = system32.join("ping.exe");
        let new_nd300 = system32.join("where.exe");
        let new_speedqx = system32.join("whoami.exe");
        let nd300 = dir.join("nd300.exe");
        let speedqx = dir.join("speedqx.exe");
        std::fs::copy(&old_image, &nd300).expect("copy live nd300 fixture");
        std::fs::copy(&old_image, &speedqx).expect("copy speedqx fixture");
        let old_bytes = std::fs::read(&old_image).expect("read old fixture image");

        let child = Command::new(&nd300)
            .args(["-t", "127.0.0.1"])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("start live nd300 fixture");
        let mut child = ChildGuard(child);

        let retirement =
            RunningImageRetirement::prepare_in_dir(&dir, "9.8.7").expect("retire live binary pair");
        assert!(
            child.0.try_wait().expect("query live fixture").is_none(),
            "retirement terminated the running image"
        );
        assert!(!nd300.exists(), "canonical nd300 path was not released");
        assert!(
            dir.join("nd300.update-old-9.8.7.exe").exists(),
            "running image was not preserved"
        );

        std::fs::copy(&new_nd300, &nd300).expect("install replacement nd300 fixture");
        std::fs::copy(&new_speedqx, &speedqx).expect("install replacement speedqx fixture");
        retirement.finish(false).expect("restore the original pair");

        assert!(
            child
                .0
                .try_wait()
                .expect("query restored fixture")
                .is_none(),
            "rollback terminated the running image"
        );
        assert_eq!(
            std::fs::read(&nd300).expect("read restored nd300"),
            old_bytes,
            "rollback did not restore nd300.exe exactly"
        );
        assert_eq!(
            std::fs::read(&speedqx).expect("read restored speedqx"),
            old_bytes,
            "rollback did not restore speedqx.exe exactly"
        );
        assert!(
            !dir.join("nd300.update-old-9.8.7.exe").exists()
                && !dir.join("speedqx.update-old-9.8.7.exe").exists(),
            "rollback left a retired image behind"
        );

        drop(child);
        std::fs::remove_dir_all(&dir).expect("remove running-image fixture directory");
    }

    #[test]
    fn windows_cargo_route_never_falls_back_to_the_standalone_installer() {
        assert_eq!(
            windows_cargo_or_installer_plan(true, true).expect("Cargo route"),
            vec![UpdateStrategy::Cargo]
        );
        assert!(windows_cargo_or_installer_plan(true, false).is_err());
    }

    #[test]
    fn windows_standalone_route_never_migrates_to_cargo() {
        assert_eq!(
            windows_cargo_or_installer_plan(false, true).expect("standalone route"),
            vec![
                UpdateStrategy::InstallerPowerShell,
                UpdateStrategy::InstallerPwsh,
            ]
        );
    }

    #[test]
    fn windows_cargo_registration_requires_an_exact_nd300_registry_key() {
        use std::time::{SystemTime, UNIX_EPOCH};

        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock before Unix epoch")
            .as_nanos();
        let home = std::env::temp_dir().join(format!(
            "nd300-cargo-registration-{}-{unique}",
            std::process::id()
        ));
        std::fs::create_dir_all(&home).expect("create Cargo registry fixture");

        std::fs::write(
            home.join(".crates2.json"),
            r#"{"installs":{"unrelated 1.0.0 (registry+https://example.invalid)":{}}}"#,
        )
        .expect("write unrelated Cargo registry");
        assert!(!windows_cargo_package_registered_at(&home));

        std::fs::write(
            home.join(".crates2.json"),
            r#"{"installs":{"nd300 3.6.3 (registry+https://github.com/rust-lang/crates.io-index)":{}}}"#,
        )
        .expect("write ND300 Cargo registry");
        assert!(windows_cargo_package_registered_at(&home));

        std::fs::remove_file(home.join(".crates2.json")).expect("remove JSON registry fixture");
        std::fs::write(
            home.join(".crates.toml"),
            "[v1]\n\"nd300 3.5.2 (registry+https://github.com/rust-lang/crates.io-index)\" = [\"nd300.exe\", \"speedqx.exe\"]\n",
        )
        .expect("write legacy Cargo registry");
        assert!(windows_cargo_package_registered_at(&home));

        std::fs::remove_dir_all(&home).expect("remove Cargo registry fixture");
    }

    #[test]
    fn json_method_maps_to_legacy_taxonomy() {
        assert_eq!(UpdateStrategy::Cargo.json_method(), "cargo");
        #[cfg(unix)]
        assert_eq!(UpdateStrategy::UnixArchive.json_method(), "installer");
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
        #[cfg(not(unix))]
        let labels = [
            UpdateStrategy::MsiGlobal.label(),
            UpdateStrategy::MsiCorporate.label(),
            UpdateStrategy::ExeGlobal.label(),
            UpdateStrategy::ExeCorporate.label(),
            UpdateStrategy::Cargo.label(),
            UpdateStrategy::InstallerPowerShell.label(),
            UpdateStrategy::InstallerPwsh.label(),
        ];
        #[cfg(unix)]
        let labels = [
            UpdateStrategy::MsiGlobal.label(),
            UpdateStrategy::MsiCorporate.label(),
            UpdateStrategy::ExeGlobal.label(),
            UpdateStrategy::ExeCorporate.label(),
            UpdateStrategy::Cargo.label(),
            UpdateStrategy::InstallerPowerShell.label(),
            UpdateStrategy::InstallerPwsh.label(),
            UpdateStrategy::UnixArchive.label(),
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
    fn exact_release_assets_are_tag_pinned_and_reject_moving_or_unsafe_versions() {
        assert_eq!(
            exact_release_asset_url("3.6.3", MSI_GLOBAL_ASSET).expect("valid exact URL"),
            "https://github.com/QubeTX/qube-network-diagnostics/releases/download/v3.6.3/nd300-x86_64-pc-windows-msvc.msi"
        );
        for invalid in ["latest", "v3.6.3", "3.6.3/other", "3.6", "3.6.3-rc.1"] {
            assert!(exact_release_asset_url(invalid, MSI_GLOBAL_ASSET).is_err());
        }
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
            target_retained: false,
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
        assert_eq!(
            classify_install_path(r"C:\Program Files\nd-300\bin\nd300.exe"),
            InstallOrigin::MsiGlobal,
            "pre-v3.1 Global MSI installs remain classifiable",
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

    #[cfg(windows)]
    #[test]
    fn cargo_path_overrides_stale_installer_marker_and_owner() {
        assert_eq!(
            resolve_install_origin(
                InstallOrigin::CargoOrInstaller,
                Some(InstallOrigin::ExeGlobal),
                Some(InstallOrigin::MsiGlobal),
            ),
            InstallOrigin::CargoOrInstaller,
        );
    }

    #[cfg(windows)]
    #[test]
    fn registered_owner_overrides_contradictory_marker() {
        assert_eq!(
            resolve_install_origin(
                InstallOrigin::MsiGlobal,
                Some(InstallOrigin::ExeGlobal),
                Some(InstallOrigin::MsiGlobal),
            ),
            InstallOrigin::MsiGlobal,
        );
    }

    #[cfg(windows)]
    #[test]
    fn registered_record_scope_accepts_corporate_but_protects_global() {
        assert_eq!(
            msi_origin_for_registered_record("nd300 (Corporate Edition)", false),
            Some(InstallOrigin::MsiCorporate),
        );
        assert_eq!(
            msi_origin_for_registered_record("nd300 (Corporate Edition)", true),
            Some(InstallOrigin::MsiCorporate),
        );
        assert_eq!(
            msi_origin_for_registered_record("nd300", true),
            Some(InstallOrigin::MsiGlobal),
        );
        assert_eq!(
            msi_origin_for_registered_record("nd-300", true),
            Some(InstallOrigin::MsiGlobal),
            "pre-v3.1 Global MSI display name remains recognized",
        );
        assert_eq!(msi_origin_for_registered_record("nd300", false), None);
        assert_eq!(msi_origin_for_registered_record("nd-300", false), None);
    }

    #[cfg(windows)]
    #[test]
    fn corporate_msi_hkcu_record_owns_its_installed_binary() {
        use winreg::enums::{HKEY_CURRENT_USER, KEY_WOW64_32KEY, KEY_WRITE};
        use winreg::RegKey;

        const PRODUCT_CODE: &str = "{A1111111-B222-4CCC-8DDD-E55555555555}";
        const UNINSTALL_ROOT: &str = "Software\\Microsoft\\Windows\\CurrentVersion\\Uninstall";

        let install_location =
            std::env::temp_dir().join(format!("nd300-corporate-owner-test-{}", std::process::id()));
        let bin_dir = install_location.join("bin");
        std::fs::create_dir_all(&bin_dir).expect("create test install directory");
        let executable = bin_dir.join("nd300.exe");
        std::fs::write(&executable, b"test").expect("create test executable");

        let hkcu = RegKey::predef(HKEY_CURRENT_USER);
        let key_path = format!("{UNINSTALL_ROOT}\\{PRODUCT_CODE}");
        let _ = hkcu.delete_subkey_with_flags(&key_path, KEY_WOW64_32KEY);
        let (key, _) = hkcu
            .create_subkey_with_flags(&key_path, KEY_WRITE | KEY_WOW64_32KEY)
            .expect("create test ARP record");
        key.set_value("DisplayName", &"nd300 (Corporate Edition)")
            .expect("write DisplayName");
        key.set_value(
            "InstallLocation",
            &install_location.to_string_lossy().as_ref(),
        )
        .expect("write InstallLocation");
        key.set_value("WindowsInstaller", &1_u32)
            .expect("write WindowsInstaller");
        drop(key);

        let executable = executable
            .canonicalize()
            .expect("canonicalize test executable");
        let owner = registered_install_owner(&executable);

        hkcu.delete_subkey_with_flags(&key_path, KEY_WOW64_32KEY)
            .expect("remove test ARP record");
        std::fs::remove_dir_all(&install_location).expect("remove test install directory");

        assert_eq!(
            owner,
            Some(RegisteredInstallOwner {
                origin: InstallOrigin::MsiCorporate,
                install_location,
                uninstall: RegisteredUninstall::Msi {
                    product_code: PRODUCT_CODE.to_string(),
                },
                machine_hive: false,
            })
        );
    }

    #[cfg(windows)]
    #[test]
    fn marker_is_used_only_when_its_scope_matches_the_path() {
        assert_eq!(
            resolve_install_origin(
                InstallOrigin::MsiGlobal,
                Some(InstallOrigin::ExeGlobal),
                None,
            ),
            InstallOrigin::ExeGlobal,
        );
        assert_eq!(
            resolve_install_origin(
                InstallOrigin::MsiGlobal,
                Some(InstallOrigin::ExeCorporate),
                None,
            ),
            InstallOrigin::MsiGlobal,
        );
        assert_eq!(
            resolve_install_origin(InstallOrigin::Unknown, Some(InstallOrigin::MsiGlobal), None,),
            InstallOrigin::Unknown,
        );
    }

    #[cfg(windows)]
    #[test]
    fn product_code_validation_accepts_only_a_bare_guid() {
        assert_eq!(
            normalize_product_code("{95739c02-d3ee-45f5-a3b1-7bf1fc4fea59}").as_deref(),
            Some("{95739C02-D3EE-45F5-A3B1-7BF1FC4FEA59}"),
        );
        assert_eq!(normalize_product_code("msiexec /x {bad}"), None);
        assert_eq!(
            normalize_product_code("{95739C02-D3EE-45F5-A3B1-7BF1FC4FEA5Z}"),
            None
        );
    }

    #[cfg(windows)]
    #[test]
    fn inno_uninstaller_must_live_under_registered_install_location() {
        let root = Path::new(r"C:\Program Files\nd300");
        assert_eq!(
            validated_inno_uninstaller(r#""C:\Program Files\nd300\unins000.exe""#, root,),
            Some(root.join("unins000.exe")),
        );
        assert_eq!(
            validated_inno_uninstaller(r#""C:\Windows\System32\cmd.exe" /c whoami"#, root),
            None,
        );
    }
}
