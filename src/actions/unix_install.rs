//! Fail-closed Unix install/update primitives.
//!
//! The public `update` and `uninstall` actions use this module to keep path and
//! package-origin decisions in one place. Nothing here guesses that an unknown
//! executable is safe to replace or delete.

#![cfg(unix)]

use crate::platform::invoking_user::InvokingUser;
use crate::VERSION;
use sha2::{Digest, Sha256};
use std::ffi::{CString, OsStr};
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::os::fd::{AsRawFd, FromRawFd, RawFd};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt};
use std::path::{Component, Path, PathBuf};
use std::process::Stdio;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::io::AsyncWriteExt;

use super::uninstall::CleanupReport;

const RELEASE_BASE: &str = "https://github.com/QubeTX/qube-network-diagnostics/releases/download";
const MAX_ARCHIVE_BYTES: usize = 128 * 1024 * 1024;
const MAX_SIDECAR_BYTES: usize = 8 * 1024;
const MAX_BINARY_BYTES: u64 = 96 * 1024 * 1024;
const MAX_UNCOMPRESSED_BYTES: u64 = 256 * 1024 * 1024;
const MAX_ARCHIVE_ENTRIES: usize = 256;
const COMMAND_TIMEOUT: Duration = Duration::from_secs(15);
const CARGO_TIMEOUT: Duration = Duration::from_secs(15 * 60);
#[cfg(target_os = "macos")]
const MACOS_TEAM_ID: &str = "M9D5379H93";
#[cfg(target_os = "macos")]
const MACOS_LEAF_SHA1: &str = "739B04530883FF9B665C66BD464F98C622971B32";
const TRANSACTION_MARKER: &str = ".nd300-update-transaction-v1";
const COMMIT_MARKER_STAGE: &str = ".nd300-update-transaction-v1-commit";
const NEW_ND300: &str = ".nd300-update-new-nd300";
const NEW_SPEEDQX: &str = ".nd300-update-new-speedqx";
const BACKUP_ND300: &str = ".nd300-update-backup-nd300";
const BACKUP_SPEEDQX: &str = ".nd300-update-backup-speedqx";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum UnixOriginKind {
    Cargo,
    ManagedArchive,
    Symlink,
    PackageManager,
    LocalBuild,
    Unknown,
}

#[derive(Debug)]
pub(crate) struct UnixInstallOrigin {
    pub(crate) kind: UnixOriginKind,
    pub(crate) executable: PathBuf,
    pub(crate) invocation_symlink: Option<PathBuf>,
}

/// Detect the origin of the running binary without trusting `argv[0]` blindly.
pub(crate) fn detect_origin(user: &InvokingUser) -> io::Result<UnixInstallOrigin> {
    let executable = std::env::current_exe()?.canonicalize()?;
    let invocation_symlink = invocation_symlink_to(&executable);
    if let Some(link) = invocation_symlink {
        let kind = if is_package_manager_path(&link) || is_package_manager_path(&executable) {
            UnixOriginKind::PackageManager
        } else {
            UnixOriginKind::Symlink
        };
        return Ok(UnixInstallOrigin {
            kind,
            executable,
            invocation_symlink: Some(link),
        });
    }

    let cargo_bin = user.cargo_home().join("bin");
    let receipt_valid = cargo_dist_receipt_valid(user);
    let kind = classify_origin_path(
        &executable,
        &cargo_bin,
        cargo_package_registered(&user.cargo_home()),
        receipt_valid,
    );
    Ok(UnixInstallOrigin {
        kind,
        executable,
        invocation_symlink: None,
    })
}

pub(crate) fn classify_origin_path(
    executable: &Path,
    cargo_bin: &Path,
    cargo_registered: bool,
    cargo_dist_receipt_valid: bool,
) -> UnixOriginKind {
    if looks_like_local_build(executable) {
        return UnixOriginKind::LocalBuild;
    }
    if is_package_manager_path(executable) {
        return UnixOriginKind::PackageManager;
    }
    if executable
        .parent()
        .is_some_and(|parent| same_path(parent, cargo_bin))
    {
        return match (cargo_registered, cargo_dist_receipt_valid) {
            (true, false) => UnixOriginKind::Cargo,
            (false, true) => UnixOriginKind::ManagedArchive,
            // Two conflicting owners are not evidence for either one. Refuse
            // automatic mutation and direct the user to the install page.
            (true, true) | (false, false) => UnixOriginKind::Unknown,
        };
    }
    UnixOriginKind::Unknown
}

pub(crate) fn cargo_dist_receipt_valid(user: &InvokingUser) -> bool {
    capture_validated_receipt(user).is_some()
}

fn cargo_dist_receipt_path(user: &InvokingUser) -> PathBuf {
    let config_home = if !user.is_different_from_effective_user() {
        std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .filter(|path| path.is_absolute())
            .unwrap_or_else(|| user.home().join(".config"))
    } else {
        user.home().join(".config")
    };
    config_home.join("nd300").join("nd300-receipt.json")
}

fn validate_cargo_dist_receipt(bytes: &[u8], cargo_home: &Path) -> bool {
    let Ok(receipt) = serde_json::from_slice::<serde_json::Value>(bytes) else {
        return false;
    };
    let expected_prefix = cargo_home
        .canonicalize()
        .unwrap_or_else(|_| cargo_home.to_path_buf());
    let Some(prefix) = receipt
        .get("install_prefix")
        .and_then(serde_json::Value::as_str)
    else {
        return false;
    };
    let receipt_prefix = PathBuf::from(prefix);
    let receipt_prefix = receipt_prefix.canonicalize().unwrap_or(receipt_prefix);
    let binaries = receipt
        .get("binaries")
        .and_then(serde_json::Value::as_array)
        .map(|values| {
            values
                .iter()
                .filter_map(serde_json::Value::as_str)
                .collect::<Vec<_>>()
        });
    receipt
        .pointer("/provider/source")
        .and_then(serde_json::Value::as_str)
        == Some("cargo-dist")
        && receipt
            .pointer("/source/app_name")
            .and_then(serde_json::Value::as_str)
            == Some("nd300")
        && receipt
            .pointer("/source/name")
            .and_then(serde_json::Value::as_str)
            == Some("qube-network-diagnostics")
        && receipt
            .pointer("/source/owner")
            .and_then(serde_json::Value::as_str)
            == Some("QubeTX")
        && receipt
            .get("install_layout")
            .and_then(serde_json::Value::as_str)
            == Some("cargo-home")
        && receipt_prefix.is_absolute()
        && receipt_prefix == expected_prefix
        && binaries.as_deref() == Some(&["nd300", "speedqx"])
}

#[derive(Debug, Clone)]
struct ValidatedReceipt {
    path: PathBuf,
    bytes: Vec<u8>,
}

fn capture_validated_receipt(user: &InvokingUser) -> Option<ValidatedReceipt> {
    let path = cargo_dist_receipt_path(user);
    if path_has_symlink_component(&path) {
        return None;
    }
    let metadata = fs::symlink_metadata(&path).ok()?;
    if !metadata.file_type().is_file() {
        return None;
    }
    let bytes = read_limited(&path, 256 * 1024).ok()?;
    validate_cargo_dist_receipt(&bytes, &user.cargo_home())
        .then_some(ValidatedReceipt { path, bytes })
}

fn remove_captured_receipt(receipt: &ValidatedReceipt, cargo_home: &Path) -> Result<bool, String> {
    if path_has_symlink_component(&receipt.path) {
        return Err(format!(
            "receipt path changed to or traverses a symlink: {}",
            receipt.path.display()
        ));
    }
    let metadata = fs::symlink_metadata(&receipt.path)
        .map_err(|error| format!("inspect {}: {error}", receipt.path.display()))?;
    if !metadata.file_type().is_file() {
        return Err(format!(
            "receipt is no longer a regular file: {}",
            receipt.path.display()
        ));
    }
    let current = read_limited(&receipt.path, 256 * 1024)
        .map_err(|error| format!("read {}: {error}", receipt.path.display()))?;
    if current != receipt.bytes || !validate_cargo_dist_receipt(&current, cargo_home) {
        return Err(format!(
            "receipt changed after origin validation; refusing to delete {}",
            receipt.path.display()
        ));
    }
    fs::remove_file(&receipt.path)
        .map_err(|error| format!("remove {}: {error}", receipt.path.display()))?;

    // Never recursively delete this user directory. `remove_dir` succeeds only
    // when the exact receipt was its final entry, preserving any unknown files.
    if let Some(parent) = receipt.path.parent() {
        let _ = fs::remove_dir(parent);
    }
    Ok(true)
}

fn path_has_symlink_component(path: &Path) -> bool {
    let mut current = PathBuf::new();
    for component in path.components() {
        current.push(component.as_os_str());
        if fs::symlink_metadata(&current)
            .ok()
            .is_some_and(|metadata| metadata.file_type().is_symlink())
        {
            return true;
        }
    }
    false
}

fn invocation_symlink_to(executable: &Path) -> Option<PathBuf> {
    let argv0 = std::env::args_os().next()?;
    let candidate = resolve_invocation_candidate(&argv0)?;
    if !fs::symlink_metadata(&candidate)
        .ok()?
        .file_type()
        .is_symlink()
    {
        return None;
    }
    let target = candidate.canonicalize().ok()?;
    if same_path(&target, executable) {
        Some(candidate)
    } else {
        None
    }
}

fn resolve_invocation_candidate(argv0: &OsStr) -> Option<PathBuf> {
    let raw = PathBuf::from(argv0);
    if raw.components().count() > 1 || raw.is_absolute() {
        return Some(if raw.is_absolute() {
            raw
        } else {
            std::env::current_dir().ok()?.join(raw)
        });
    }
    std::env::var_os("PATH")
        .into_iter()
        .flat_map(|path| std::env::split_paths(&path).collect::<Vec<_>>())
        .map(|dir| dir.join(&raw))
        .find(|candidate| fs::symlink_metadata(candidate).is_ok())
}

fn cargo_package_registered(cargo_home: &Path) -> bool {
    let json_path = cargo_home.join(".crates2.json");
    if let Ok(bytes) = read_limited(&json_path, 2 * 1024 * 1024) {
        if let Ok(value) = serde_json::from_slice::<serde_json::Value>(&bytes) {
            if value
                .get("installs")
                .and_then(serde_json::Value::as_object)
                .is_some_and(|installs| installs.keys().any(|key| cargo_key_is_nd300(key)))
            {
                return true;
            }
        }
    }

    let legacy = cargo_home.join(".crates.toml");
    read_limited(&legacy, 2 * 1024 * 1024)
        .ok()
        .and_then(|bytes| String::from_utf8(bytes).ok())
        .is_some_and(|text| text.lines().any(|line| cargo_key_is_nd300(line.trim())))
}

fn cargo_key_is_nd300(key: &str) -> bool {
    key.trim_start_matches(['"', '\''])
        .strip_prefix("nd300 ")
        .is_some_and(|rest| rest.chars().next().is_some_and(|c| c.is_ascii_digit()))
}

fn read_limited(path: &Path, limit: u64) -> io::Result<Vec<u8>> {
    let file = File::open(path)?;
    if file.metadata()?.len() > limit {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "file too large"));
    }
    let mut bytes = Vec::new();
    file.take(limit + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > limit {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "file too large"));
    }
    Ok(bytes)
}

fn looks_like_local_build(path: &Path) -> bool {
    path.parent()
        .and_then(Path::file_name)
        .and_then(OsStr::to_str)
        .is_some_and(|name| matches!(name, "debug" | "release"))
        && path
            .parent()
            .and_then(Path::parent)
            .and_then(Path::file_name)
            .and_then(OsStr::to_str)
            == Some("target")
}

fn is_package_manager_path(path: &Path) -> bool {
    let text = path.to_string_lossy();
    [
        "/opt/homebrew/",
        "/usr/local/Cellar/",
        "/home/linuxbrew/.linuxbrew/",
        "/nix/store/",
        "/opt/local/",
        "/snap/",
        "/usr/bin/",
        "/usr/sbin/",
        "/bin/",
        "/sbin/",
    ]
    .iter()
    .any(|prefix| text.starts_with(prefix))
}

fn same_path(left: &Path, right: &Path) -> bool {
    left.canonicalize().unwrap_or_else(|_| left.to_path_buf())
        == right.canonicalize().unwrap_or_else(|_| right.to_path_buf())
}

pub(crate) fn cargo_executable(user: &InvokingUser) -> Option<PathBuf> {
    let user_cargo = user.cargo_home().join("bin").join("cargo");
    select_cargo_executable(&user_cargo, user.is_different_from_effective_user())
}

fn select_cargo_executable(user_cargo: &Path, elevated_for_another_user: bool) -> Option<PathBuf> {
    if user_cargo.is_file() {
        Some(user_cargo.to_path_buf())
    } else if elevated_for_another_user {
        // An elevated process must never inherit root's PATH and accidentally
        // update root's Cargo installation on behalf of the invoking user.
        None
    } else {
        Some(PathBuf::from("cargo"))
    }
}

fn cargo_install_args(latest: &str) -> Vec<&str> {
    vec![
        "install",
        "nd300",
        "--version",
        latest,
        "--force",
        "--locked",
    ]
}

pub(crate) async fn cargo_is_invokable(user: &InvokingUser) -> bool {
    let Some(cargo) = cargo_executable(user) else {
        return false;
    };
    let mut command = tokio::process::Command::new(cargo);
    command.arg("--version");
    command.stdin(Stdio::null());
    command.stdout(Stdio::null());
    command.stderr(Stdio::null());
    command.env("HOME", user.home());
    command.env("CARGO_HOME", user.cargo_home());
    user.configure_command(&mut command);
    command.kill_on_drop(true);
    matches!(
        tokio::time::timeout(COMMAND_TIMEOUT, command.status()).await,
        Ok(Ok(status)) if status.success()
    )
}

pub(crate) async fn cargo_install(latest: &str, user: &InvokingUser) -> Result<(), String> {
    validate_release_version(latest)?;
    let cargo = cargo_executable(user).ok_or_else(|| {
        format!(
            "the invoking user's Cargo executable was not found at {}",
            user.cargo_home().join("bin").join("cargo").display()
        )
    })?;
    let legacy_receipt = capture_validated_receipt(user);
    let mut command = tokio::process::Command::new(cargo);
    command.args(cargo_install_args(latest));
    command.stdin(Stdio::null());
    command.stdout(Stdio::piped());
    command.stderr(Stdio::piped());
    command.env("HOME", user.home());
    command.env("CARGO_HOME", user.cargo_home());
    user.configure_command(&mut command);
    command.kill_on_drop(true);

    let output = tokio::time::timeout(CARGO_TIMEOUT, command.output())
        .await
        .map_err(|_| "cargo install exceeded the 15-minute safety deadline".to_string())?
        .map_err(|error| format!("failed to launch cargo: {error}"))?;
    if !output.status.success() {
        return Err(format!(
            "cargo exited with code {}; {}",
            output.status.code().unwrap_or(-1),
            summarize_output(&output.stdout, &output.stderr)
        ));
    }

    verify_installed_pair(&user.cargo_home().join("bin"), latest, user).await?;
    if let Some(receipt) = legacy_receipt {
        if let Err(error) = remove_captured_receipt(&receipt, &user.cargo_home()) {
            eprintln!(
                "  · warning: Cargo installed and verified both binaries, but the validated legacy cargo-dist receipt was retained: {error}"
            );
        }
    }
    Ok(())
}

pub(crate) async fn cargo_uninstall(user: &InvokingUser) -> Result<CleanupReport, String> {
    let cargo = cargo_executable(user).ok_or_else(|| {
        format!(
            "the invoking user's Cargo executable was not found at {}",
            user.cargo_home().join("bin").join("cargo").display()
        )
    })?;
    let legacy_receipt = capture_validated_receipt(user);
    let mut command = tokio::process::Command::new(cargo);
    command.args(["uninstall", "nd300"]);
    command.stdin(Stdio::null());
    command.stdout(Stdio::piped());
    command.stderr(Stdio::piped());
    command.env("HOME", user.home());
    command.env("CARGO_HOME", user.cargo_home());
    user.configure_command(&mut command);
    command.kill_on_drop(true);

    let output = tokio::time::timeout(CARGO_TIMEOUT, command.output())
        .await
        .map_err(|_| "cargo uninstall exceeded the 15-minute safety deadline".to_string())?
        .map_err(|error| format!("failed to launch cargo: {error}"))?;
    if !output.status.success() {
        return Err(format!(
            "cargo uninstall exited with code {}; {}",
            output.status.code().unwrap_or(-1),
            summarize_output(&output.stdout, &output.stderr)
        ));
    }

    let bin = user.cargo_home().join("bin");
    let nd300_gone = !bin.join("nd300").exists();
    let speedqx_gone = !bin.join("speedqx").exists();
    if !nd300_gone || !speedqx_gone {
        return Err("cargo reported success but one or both package binaries remain".to_string());
    }
    let mut notes = vec!["Removed through `cargo uninstall nd300`".to_string()];
    let receipt_removed = if let Some(receipt) = legacy_receipt {
        match remove_captured_receipt(&receipt, &user.cargo_home()) {
            Ok(removed) => removed,
            Err(error) => {
                notes.push(format!(
                    "Binaries were removed, but the validated legacy receipt was retained: {error}"
                ));
                false
            }
        }
    } else {
        false
    };
    Ok(CleanupReport {
        binary_removed: true,
        binary_removal_scheduled: false,
        target_retained: false,
        sibling_removed: true,
        receipt_removed,
        path_cleaned: false,
        notes,
    })
}

pub(crate) async fn uninstall_detected(user: &InvokingUser) -> CleanupReport {
    let origin = match detect_origin(user) {
        Ok(origin) => origin,
        Err(error) => {
            return refused_cleanup(format!("Could not classify install origin: {error}"))
        }
    };
    match origin.kind {
        UnixOriginKind::Symlink => {
            let Some(link) = origin.invocation_symlink else {
                return refused_cleanup("Internal symlink classification error".to_string());
            };
            match fs::remove_file(&link) {
                Ok(()) => CleanupReport {
                    binary_removed: false,
                    binary_removal_scheduled: false,
                    target_retained: true,
                    sibling_removed: false,
                    receipt_removed: false,
                    path_cleaned: false,
                    notes: vec![format!(
                        "Removed invocation symlink {}; target installation was left intact",
                        link.display()
                    )],
                },
                Err(error) => refused_cleanup(format!(
                    "Could not remove invocation symlink {}: {error}",
                    link.display()
                )),
            }
        }
        UnixOriginKind::Cargo => match cargo_uninstall(user).await {
            Ok(report) => report,
            Err(error) => refused_cleanup(error),
        },
        UnixOriginKind::ManagedArchive => uninstall_managed_archive(user, &origin),
        UnixOriginKind::PackageManager => refused_cleanup(format!(
            "Refusing to modify package-manager-owned installation at {}; use that package manager to uninstall ND300",
            origin.executable.display()
        )),
        UnixOriginKind::LocalBuild => refused_cleanup(format!(
            "Refusing to uninstall a local build at {}",
            origin.executable.display()
        )),
        UnixOriginKind::Unknown => refused_cleanup(format!(
            "Refusing to remove unknown installation at {}; remove it through its original installer",
            origin.executable.display()
        )),
    }
}

fn uninstall_managed_archive(user: &InvokingUser, origin: &UnixInstallOrigin) -> CleanupReport {
    let Some(receipt) = capture_validated_receipt(user) else {
        return refused_cleanup(
            "Validated cargo-dist receipt disappeared before uninstall; no files were removed"
                .to_string(),
        );
    };
    let bin = user.cargo_home().join("bin");
    let nd300 = bin.join("nd300");
    let speedqx = bin.join("speedqx");
    if !same_path(&origin.executable, &nd300) {
        return refused_cleanup(format!(
            "Validated receipt does not own the running executable at {}; no files were removed",
            origin.executable.display()
        ));
    }
    let nd300_regular = fs::symlink_metadata(&nd300)
        .ok()
        .is_some_and(|metadata| metadata.file_type().is_file());
    if !nd300_regular || path_has_symlink_component(&nd300) {
        return refused_cleanup(format!(
            "Expected managed binary is missing, non-regular, or traverses a symlink: {}; no files were removed",
            nd300.display()
        ));
    }
    let speedqx_present = match fs::symlink_metadata(&speedqx) {
        Ok(metadata) if metadata.file_type().is_file() && !path_has_symlink_component(&speedqx) => {
            true
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => false,
        _ => {
            return refused_cleanup(format!(
                "Expected managed sibling is non-regular or traverses a symlink: {}; no files were removed",
                speedqx.display()
            ));
        }
    };

    // Keep the running nd300 command available until its sibling has been
    // removed successfully. The validated receipt is removed last, so a failed
    // binary deletion remains classifiable and retryable.
    if speedqx_present {
        if let Err(error) = fs::remove_file(&speedqx) {
            return refused_cleanup(format!(
                "Could not remove managed binary {}: {error}; nd300 and its receipt were retained",
                speedqx.display()
            ));
        }
    }
    if let Err(error) = fs::remove_file(&nd300) {
        return CleanupReport {
            binary_removed: false,
            binary_removal_scheduled: false,
            target_retained: false,
            sibling_removed: speedqx_present,
            receipt_removed: false,
            path_cleaned: false,
            notes: vec![format!(
                "Removed speedqx, but could not remove {}: {error}; the validated receipt was retained for a safe retry",
                nd300.display()
            )],
        };
    }

    let mut notes = vec![format!(
        "Removed validated cargo-dist binaries from {}",
        bin.display()
    )];
    let receipt_removed = match remove_captured_receipt(&receipt, &user.cargo_home()) {
        Ok(removed) => removed,
        Err(error) => {
            notes.push(format!(
                "Binaries were removed, but the validated receipt was retained: {error}"
            ));
            false
        }
    };
    CleanupReport {
        binary_removed: true,
        binary_removal_scheduled: false,
        target_retained: false,
        sibling_removed: speedqx_present,
        receipt_removed,
        path_cleaned: false,
        notes,
    }
}

fn refused_cleanup(message: String) -> CleanupReport {
    CleanupReport {
        binary_removed: false,
        binary_removal_scheduled: false,
        target_retained: false,
        sibling_removed: false,
        receipt_removed: false,
        path_cleaned: false,
        notes: vec![message],
    }
}

pub(crate) async fn update_from_archive(latest: &str, user: &InvokingUser) -> Result<(), String> {
    validate_release_version(latest)?;
    let origin = detect_origin(user).map_err(|error| format!("install origin: {error}"))?;
    // A symlink is a safe uninstall target in its own right, but it must not
    // turn an unknown target into an updateable installation. Re-classify the
    // canonical target and require the same Cargo metadata or exact cargo-dist
    // receipt that a direct invocation would require.
    let update_kind = effective_update_kind_for(&origin, user);
    let managed_archive = update_kind == UnixOriginKind::ManagedArchive;
    let install_dir = match update_kind {
        UnixOriginKind::ManagedArchive => {
            let parent = origin
                .executable
                .parent()
                .ok_or_else(|| "running executable has no parent directory".to_string())?;
            if !same_path(parent, &user.cargo_home().join("bin")) {
                return Err(format!(
                    "refusing to replace symlink target outside the invoking user's Cargo bin: {}",
                    origin.executable.display()
                ));
            }
            parent.to_path_buf()
        }
        UnixOriginKind::Cargo => {
            return Err(format!(
                "refusing to replace Cargo-owned installation at {} through the standalone archive updater",
                origin.executable.display()
            ));
        }
        UnixOriginKind::PackageManager => {
            return Err(format!(
                "refusing to replace package-manager-owned installation at {}",
                origin.executable.display()
            ));
        }
        UnixOriginKind::LocalBuild => {
            return Err(format!(
                "refusing to replace local build at {}",
                origin.executable.display()
            ));
        }
        UnixOriginKind::Symlink | UnixOriginKind::Unknown => {
            return Err(format!(
                "refusing to replace unknown installation at {}",
                origin.executable.display()
            ));
        }
    };

    // Pin the validated install directory once. All privileged mutations below
    // are relative to this descriptor; renaming or replacing the public path
    // cannot redirect a swap into another directory.
    let pinned_install = PinnedInstallDir::open(&install_dir, user)?;
    recover_interrupted_transaction(&pinned_install)?;
    let target = release_target().ok_or_else(|| {
        format!(
            "no prebuilt ND300 archive is available for {}-{}",
            std::env::consts::OS,
            std::env::consts::ARCH
        )
    })?;
    let asset = format!("nd300-{target}.tar.xz");
    let url = format!("{RELEASE_BASE}/v{latest}/{asset}");
    let sidecar_url = format!("{url}.sha256");
    let temp = SecureTempDir::new()?;
    let archive_path = temp.path().join(&asset);
    download_limited(&url, &archive_path, MAX_ARCHIVE_BYTES).await?;
    let sidecar = download_bytes_limited(&sidecar_url, MAX_SIDECAR_BYTES).await?;
    let expected = parse_sha256_sidecar(&sidecar, &asset)
        .ok_or_else(|| format!("malformed SHA-256 sidecar at {sidecar_url}"))?;
    let actual = compute_sha256(&archive_path)?;
    checksum_verdict(&actual, &expected)?;

    let extract_dir = temp.path().join("extract");
    fs::create_dir(&extract_dir)
        .map_err(|error| format!("failed to create extraction directory: {error}"))?;
    let extracted = extract_archive_allowlisted(&archive_path, &extract_dir, target)?;
    verify_staged_pair(&extracted, latest).await?;

    let mut transaction = PairTransaction::begin(pinned_install, &extracted)?;
    match verify_installed_archive_pair(&transaction.directory, latest, user).await {
        Ok(()) => {
            if let Some(warning) = transaction.commit()? {
                eprintln!("  · warning: {warning}");
            }
            if managed_archive {
                if let Err(error) = update_receipt_version(user, latest) {
                    eprintln!(
                        "  · warning: binaries updated and verified, but the install receipt version could not be refreshed: {error}"
                    );
                }
            }
            Ok(())
        }
        Err(error) => {
            let rollback = transaction.rollback();
            match rollback {
                Ok(()) => Err(format!(
                    "post-install verification failed and the previous binaries were restored: {error}"
                )),
                Err(rollback_error) => Err(format!(
                    "post-install verification failed ({error}); rollback also failed ({rollback_error}). Transaction marker retained at {}",
                    install_dir.join(TRANSACTION_MARKER).display()
                )),
            }
        }
    }
}

fn effective_update_kind(
    detected: UnixOriginKind,
    executable: &Path,
    cargo_bin: &Path,
    cargo_registered: bool,
    cargo_dist_receipt_valid: bool,
) -> UnixOriginKind {
    if detected == UnixOriginKind::Symlink {
        classify_origin_path(
            executable,
            cargo_bin,
            cargo_registered,
            cargo_dist_receipt_valid,
        )
    } else {
        detected
    }
}

pub(crate) fn effective_update_kind_for(
    origin: &UnixInstallOrigin,
    user: &InvokingUser,
) -> UnixOriginKind {
    effective_update_kind(
        origin.kind,
        &origin.executable,
        &user.cargo_home().join("bin"),
        cargo_package_registered(&user.cargo_home()),
        cargo_dist_receipt_valid(user),
    )
}

fn update_receipt_version(user: &InvokingUser, version: &str) -> Result<(), String> {
    let path = cargo_dist_receipt_path(user);
    if path_has_symlink_component(&path) {
        return Err(format!(
            "receipt path traverses a symlink: {}",
            path.display()
        ));
    }
    let metadata = fs::symlink_metadata(&path)
        .map_err(|error| format!("inspect {}: {error}", path.display()))?;
    if !metadata.file_type().is_file() {
        return Err(format!("receipt is not a regular file: {}", path.display()));
    }
    let bytes = read_limited(&path, 256 * 1024)
        .map_err(|error| format!("read {}: {error}", path.display()))?;
    if !validate_cargo_dist_receipt(&bytes, &user.cargo_home()) {
        return Err(format!(
            "{} no longer passes origin validation",
            path.display()
        ));
    }
    let mut receipt: serde_json::Value = serde_json::from_slice(&bytes)
        .map_err(|error| format!("parse {}: {error}", path.display()))?;
    receipt["version"] = serde_json::Value::String(version.to_string());
    let updated = serde_json::to_vec(&receipt)
        .map_err(|error| format!("serialize {}: {error}", path.display()))?;
    user.write_private_atomic(&path, &updated)
        .map_err(|error| format!("write {}: {error}", path.display()))
}

fn validate_release_version(version: &str) -> Result<(), String> {
    if version.len() > 32 {
        return Err("release version is unreasonably long".to_string());
    }
    let core = version_core(version);
    if core != version {
        return Err(format!(
            "release version must be a stable numeric tag: {version:?}"
        ));
    }
    let mut parts = core.split('.');
    let valid = (0..3).all(|_| {
        parts
            .next()
            .is_some_and(|part| !part.is_empty() && part.chars().all(|c| c.is_ascii_digit()))
    }) && parts.next().is_none();
    if valid {
        Ok(())
    } else {
        Err(format!("invalid release version {version:?}"))
    }
}

fn release_target() -> Option<&'static str> {
    if cfg!(all(target_os = "macos", target_arch = "aarch64")) {
        Some("aarch64-apple-darwin")
    } else if cfg!(all(target_os = "macos", target_arch = "x86_64")) {
        Some("x86_64-apple-darwin")
    } else if cfg!(all(
        target_os = "linux",
        target_arch = "aarch64",
        target_env = "gnu"
    )) {
        Some("aarch64-unknown-linux-gnu")
    } else if cfg!(all(
        target_os = "linux",
        target_arch = "x86_64",
        target_env = "musl"
    )) {
        Some("x86_64-unknown-linux-musl")
    } else if cfg!(all(
        target_os = "linux",
        target_arch = "x86_64",
        target_env = "gnu"
    )) {
        Some("x86_64-unknown-linux-gnu")
    } else {
        None
    }
}

async fn download_limited(url: &str, path: &Path, limit: usize) -> Result<(), String> {
    let bytes = download_bytes_limited(url, limit).await?;
    let mut options = tokio::fs::OpenOptions::new();
    options.write(true).create_new(true).mode(0o600);
    let mut file = options
        .open(path)
        .await
        .map_err(|error| format!("failed to create {}: {error}", path.display()))?;
    file.write_all(&bytes)
        .await
        .map_err(|error| format!("failed to write {}: {error}", path.display()))?;
    file.sync_all()
        .await
        .map_err(|error| format!("failed to sync {}: {error}", path.display()))
}

async fn download_bytes_limited(url: &str, limit: usize) -> Result<Vec<u8>, String> {
    let client = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(15))
        .timeout(Duration::from_secs(120))
        .build()
        .map_err(|error| format!("HTTP client: {error}"))?;
    let mut response = client
        .get(url)
        .header("User-Agent", format!("nd300/{VERSION}"))
        .send()
        .await
        .map_err(|error| format!("request failed for {url}: {error}"))?;
    if !response.status().is_success() {
        return Err(format!(
            "download returned HTTP {} for {url}",
            response.status()
        ));
    }
    if response
        .content_length()
        .is_some_and(|length| length > limit as u64)
    {
        return Err(format!("download exceeds {limit} byte safety limit: {url}"));
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|error| format!("failed reading {url}: {error}"))?
    {
        if bytes.len().saturating_add(chunk.len()) > limit {
            return Err(format!("download exceeds {limit} byte safety limit: {url}"));
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
}

fn parse_sha256_sidecar(content: &[u8], expected_asset: &str) -> Option<String> {
    let text = std::str::from_utf8(content).ok()?;
    let mut nonempty = text.lines().map(str::trim).filter(|line| !line.is_empty());
    let line = nonempty.next()?;
    if nonempty.next().is_some() {
        return None;
    }
    let separator = line.find(char::is_whitespace)?;
    let hash = &line[..separator];
    let filename = line[separator..]
        .trim()
        .strip_prefix('*')
        .unwrap_or_else(|| line[separator..].trim());
    (hash.len() == 64 && hash.chars().all(|c| c.is_ascii_hexdigit()) && filename == expected_asset)
        .then(|| hash.to_ascii_lowercase())
}

fn compute_sha256(path: &Path) -> Result<String, String> {
    let mut file =
        File::open(path).map_err(|error| format!("failed to open {}: {error}", path.display()))?;
    let mut hasher = Sha256::new();
    io::copy(&mut file, &mut hasher)
        .map_err(|error| format!("failed to hash {}: {error}", path.display()))?;
    Ok(format!("{:x}", hasher.finalize()))
}

fn checksum_verdict(actual: &str, expected: &str) -> Result<(), String> {
    if actual.eq_ignore_ascii_case(expected) {
        Ok(())
    } else {
        Err(format!(
            "SHA-256 mismatch; refusing archive (expected {expected}, got {actual})"
        ))
    }
}

#[derive(Debug)]
struct ExtractedPair {
    nd300: PathBuf,
    speedqx: PathBuf,
}

fn extract_archive_allowlisted(
    archive_path: &Path,
    output_dir: &Path,
    expected_root: &str,
) -> Result<ExtractedPair, String> {
    let file = File::open(archive_path)
        .map_err(|error| format!("failed to open archive {}: {error}", archive_path.display()))?;
    let decoder = xz2::read::XzDecoder::new(file);
    let mut archive = tar::Archive::new(decoder);
    let mut nd300 = None;
    let mut speedqx = None;
    let expected_dir = format!("nd300-{expected_root}");
    let mut uncompressed_bytes = 0_u64;

    for (index, entry) in archive
        .entries()
        .map_err(|error| format!("invalid tar archive: {error}"))?
        .enumerate()
    {
        if index >= MAX_ARCHIVE_ENTRIES {
            return Err("archive contains too many entries".to_string());
        }
        let mut entry = entry.map_err(|error| format!("invalid tar entry: {error}"))?;
        let path = entry
            .path()
            .map_err(|error| format!("invalid archive path: {error}"))?
            .into_owned();
        validate_archive_path(&path)?;
        let entry_type = entry.header().entry_type();
        if entry_type.is_symlink() || entry_type.is_hard_link() {
            return Err(format!(
                "archive contains forbidden link entry {}",
                path.display()
            ));
        }
        if !entry_type.is_file() {
            continue;
        }
        let size = entry.header().size().unwrap_or(u64::MAX);
        uncompressed_bytes = uncompressed_bytes
            .checked_add(size)
            .ok_or_else(|| "archive size overflow".to_string())?;
        if uncompressed_bytes > MAX_UNCOMPRESSED_BYTES {
            return Err("archive expands beyond the safety limit".to_string());
        }

        let Some(file_name) = path.file_name().and_then(OsStr::to_str) else {
            continue;
        };
        if !matches!(file_name, "nd300" | "speedqx") {
            continue;
        }
        let parent = path.parent().and_then(Path::to_str);
        if parent != Some(expected_dir.as_str()) {
            return Err(format!(
                "unexpected binary layout {}; expected {expected_dir}/<binary>",
                path.display()
            ));
        }
        if size > MAX_BINARY_BYTES {
            return Err(format!("binary entry {} is too large", path.display()));
        }
        let slot = if file_name == "nd300" {
            &mut nd300
        } else {
            &mut speedqx
        };
        if slot.is_some() {
            return Err(format!("duplicate {file_name} entry in archive"));
        }
        let destination = output_dir.join(file_name);
        let mut options = OpenOptions::new();
        options.write(true).create_new(true).mode(0o700);
        let mut output = options
            .open(&destination)
            .map_err(|error| format!("failed to create {}: {error}", destination.display()))?;
        io::copy(&mut entry, &mut output)
            .map_err(|error| format!("failed extracting {file_name}: {error}"))?;
        output
            .sync_all()
            .map_err(|error| format!("failed syncing {file_name}: {error}"))?;
        *slot = Some(destination);
    }

    Ok(ExtractedPair {
        nd300: nd300.ok_or_else(|| "archive is missing nd300".to_string())?,
        speedqx: speedqx.ok_or_else(|| "archive is missing speedqx".to_string())?,
    })
}

fn validate_archive_path(path: &Path) -> Result<(), String> {
    if path.is_absolute()
        || path.components().any(|component| {
            matches!(
                component,
                Component::ParentDir | Component::RootDir | Component::Prefix(_)
            )
        })
    {
        return Err(format!("unsafe archive path {}", path.display()));
    }
    Ok(())
}

async fn verify_staged_pair(pair: &ExtractedPair, expected: &str) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        verify_macos_trust(&pair.nd300, "com.qubetx.nd300").await?;
        verify_macos_trust(&pair.speedqx, "com.qubetx.speedqx").await?;
    }
    // Never execute staged macOS bytes until their Developer ID signature has
    // passed the identity/team/runtime/timestamp checks above.
    verify_binary_version(&pair.nd300, "nd300", expected, None).await?;
    verify_binary_version(&pair.speedqx, "speedqx", expected, None).await?;
    Ok(())
}

async fn verify_installed_pair(
    directory: &Path,
    expected: &str,
    user: &InvokingUser,
) -> Result<(), String> {
    let nd300 = directory.join("nd300");
    let speedqx = directory.join("speedqx");
    verify_binary_version(&nd300, "nd300", expected, Some(user)).await?;
    verify_binary_version(&speedqx, "speedqx", expected, Some(user)).await?;
    Ok(())
}

async fn verify_installed_archive_pair(
    directory: &PinnedInstallDir,
    expected: &str,
    user: &InvokingUser,
) -> Result<(), String> {
    directory.ensure_public_binding()?;
    #[cfg(target_os = "macos")]
    {
        let nd300 = directory.path.join("nd300");
        let speedqx = directory.path.join("speedqx");
        verify_macos_trust(&nd300, "com.qubetx.nd300").await?;
        verify_macos_trust(&speedqx, "com.qubetx.speedqx").await?;
    }
    verify_installed_pair(&directory.path, expected, user).await?;
    // If the pathname changed while the read-only trust checks ran, rollback
    // through the pinned descriptor instead of committing an installation we
    // can no longer address by its validated public path.
    directory.ensure_public_binding()
}

async fn verify_binary_version(
    path: &Path,
    binary_name: &str,
    expected: &str,
    run_as: Option<&InvokingUser>,
) -> Result<(), String> {
    let mut command = tokio::process::Command::new(path);
    command.arg("--version");
    command.stdin(Stdio::null());
    command.stdout(Stdio::piped());
    command.stderr(Stdio::piped());
    if let Some(user) = run_as {
        // The Cargo bin directory is intentionally user-writable. Never
        // execute a pathname from it with elevated privileges, even after
        // signature checks, because the invoking user can race a replacement.
        user.configure_command(&mut command);
    }
    command.kill_on_drop(true);
    let output = tokio::time::timeout(COMMAND_TIMEOUT, command.output())
        .await
        .map_err(|_| format!("{binary_name} --version timed out"))?
        .map_err(|error| format!("failed to execute {}: {error}", path.display()))?;
    if !output.status.success() {
        return Err(format!(
            "{} --version exited with code {}",
            path.display(),
            output.status.code().unwrap_or(-1)
        ));
    }
    let installed = parse_version_output(&output.stdout)
        .ok_or_else(|| format!("could not parse version from {}", path.display()))?;
    if versions_match(&installed, expected) {
        Ok(())
    } else {
        Err(format!(
            "{binary_name} reports v{installed}; expected v{expected}"
        ))
    }
}

fn parse_version_output(output: &[u8]) -> Option<String> {
    String::from_utf8_lossy(output)
        .split_whitespace()
        .last()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn versions_match(installed: &str, expected: &str) -> bool {
    !installed.is_empty() && installed == expected
}

fn version_core(version: &str) -> &str {
    version
        .split_once(['-', '+'])
        .map_or(version, |(core, _)| core)
}

#[cfg(target_os = "macos")]
async fn verify_macos_trust(path: &Path, expected_identifier: &str) -> Result<(), String> {
    let verify = run_bounded_command(
        "/usr/bin/codesign",
        &[
            OsStr::new("--verify"),
            OsStr::new("--strict"),
            OsStr::new("--verbose=4"),
            path.as_os_str(),
        ],
    )
    .await?;
    if !verify.status.success() {
        return Err(format!(
            "codesign rejected {}: {}",
            path.display(),
            summarize_output(&verify.stdout, &verify.stderr)
        ));
    }

    let details = run_bounded_command(
        "/usr/bin/codesign",
        &[
            OsStr::new("-d"),
            OsStr::new("--verbose=4"),
            path.as_os_str(),
        ],
    )
    .await?;
    if !details.status.success() {
        return Err(format!(
            "could not inspect signature for {}",
            path.display()
        ));
    }
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&details.stdout),
        String::from_utf8_lossy(&details.stderr)
    );
    let identifier = signing_field(&text, "Identifier")
        .ok_or_else(|| format!("signature for {} has no Identifier", path.display()))?;
    let team = signing_field(&text, "TeamIdentifier")
        .ok_or_else(|| format!("signature for {} has no TeamIdentifier", path.display()))?;
    if identifier != expected_identifier {
        return Err(format!(
            "signature identifier mismatch for {}: expected {expected_identifier}, got {identifier}",
            path.display()
        ));
    }
    if team != MACOS_TEAM_ID {
        return Err(format!(
            "signature Team ID mismatch for {}: expected {MACOS_TEAM_ID}, got {team}",
            path.display()
        ));
    }

    let authority = signing_field(&text, "Authority")
        .ok_or_else(|| format!("signature for {} has no signing authority", path.display()))?;
    if !authority.starts_with("Developer ID Application:") {
        return Err(format!(
            "signature for {} is not a Developer ID Application signature: {authority}",
            path.display()
        ));
    }
    if signing_field(&text, "Timestamp").is_none() {
        return Err(format!(
            "signature for {} has no trusted timestamp",
            path.display()
        ));
    }
    if !text.contains("(runtime)") && !text.contains("Runtime Version=") {
        return Err(format!(
            "signature for {} does not enable the hardened runtime",
            path.display()
        ));
    }

    let cert_temp = SecureTempDir::new()?;
    let cert_prefix = cert_temp.path().join("embedded-cert-");
    let extracted = run_bounded_command(
        "/usr/bin/codesign",
        &[
            OsStr::new("-d"),
            OsStr::new("--extract-certificates"),
            cert_prefix.as_os_str(),
            path.as_os_str(),
        ],
    )
    .await?;
    if !extracted.status.success() {
        return Err(format!(
            "could not extract the signing certificate from {}: {}",
            path.display(),
            summarize_output(&extracted.stdout, &extracted.stderr)
        ));
    }
    let mut leaf_path = cert_prefix.into_os_string();
    leaf_path.push("0");
    let leaf = read_limited(Path::new(&leaf_path), 128 * 1024)
        .map_err(|error| format!("could not read embedded leaf certificate: {error}"))?;
    let fingerprint = sha1_fingerprint(&leaf);
    fingerprint_verdict(&fingerprint, MACOS_LEAF_SHA1)?;

    // `spctl --type execute` does not assess a standalone Mach-O CLI as an app:
    // it returns the same "valid but does not seem to be an app" rejection for
    // both Accepted-notarized and equally signed-but-unnotarized binaries. The
    // install-policy assessment is the local Gatekeeper check that
    // distinguishes those cases. Bypass assessment caches so an earlier file
    // at the same path cannot supply the verdict for these staged bytes.
    let assessment = run_bounded_command(
        "/usr/sbin/spctl",
        &[
            OsStr::new("--assess"),
            OsStr::new("--type"),
            OsStr::new("install"),
            OsStr::new("--ignore-cache"),
            OsStr::new("--no-cache"),
            OsStr::new("--verbose=4"),
            path.as_os_str(),
        ],
    )
    .await?;
    let assessment_text = summarize_output(&assessment.stdout, &assessment.stderr);
    gatekeeper_install_verdict(assessment.status.success(), &assessment_text).map_err(|error| {
        format!(
            "Gatekeeper install-policy assessment rejected {}: {error}",
            path.display()
        )
    })?;
    Ok(())
}

#[cfg(target_os = "macos")]
fn gatekeeper_install_verdict(accepted: bool, output: &str) -> Result<(), String> {
    let normalized = output.to_ascii_lowercase();
    if accepted && !normalized.contains("source=unnotarized developer id") {
        return Ok(());
    }
    Err(if output.trim().is_empty() {
        "assessment returned no evidence".to_string()
    } else {
        output.to_string()
    })
}

#[cfg(target_os = "macos")]
fn sha1_fingerprint(der: &[u8]) -> String {
    use sha1::{Digest, Sha1};
    let mut digest = Sha1::new();
    digest.update(der);
    format!("{:X}", digest.finalize())
}

#[cfg(target_os = "macos")]
fn fingerprint_verdict(actual: &str, expected: &str) -> Result<(), String> {
    if actual.eq_ignore_ascii_case(expected) {
        Ok(())
    } else {
        Err(format!(
            "Developer ID leaf certificate mismatch: expected {expected}, got {actual}"
        ))
    }
}

#[cfg(target_os = "macos")]
fn signing_field<'a>(details: &'a str, key: &str) -> Option<&'a str> {
    details
        .lines()
        .find_map(|line| line.trim().strip_prefix(key)?.strip_prefix('='))
        .map(str::trim)
        .filter(|value| !value.is_empty())
}

#[cfg(target_os = "macos")]
async fn run_bounded_command(
    launcher: &str,
    args: &[&OsStr],
) -> Result<std::process::Output, String> {
    let mut command = tokio::process::Command::new(launcher);
    command.args(args);
    command.stdin(Stdio::null());
    command.stdout(Stdio::piped());
    command.stderr(Stdio::piped());
    command.kill_on_drop(true);
    tokio::time::timeout(COMMAND_TIMEOUT, command.output())
        .await
        .map_err(|_| format!("{launcher} timed out"))?
        .map_err(|error| format!("failed to launch {launcher}: {error}"))
}

fn summarize_output(stdout: &[u8], stderr: &[u8]) -> String {
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(stdout),
        String::from_utf8_lossy(stderr)
    );
    let trimmed = combined.trim();
    if trimmed.is_empty() {
        return "no output".to_string();
    }
    trimmed
        .chars()
        .rev()
        .take(4000)
        .collect::<String>()
        .chars()
        .rev()
        .collect()
}

#[derive(Debug)]
struct PinnedInstallDir {
    path: PathBuf,
    directory: File,
    owner_uid: libc::uid_t,
    owner_gid: libc::gid_t,
    chown_created_files: bool,
}

impl PinnedInstallDir {
    fn open(path: &Path, user: &InvokingUser) -> Result<Self, String> {
        let directory = if path.strip_prefix(user.home()).is_ok() {
            user.open_directory_beneath_home(path, false)
                .map_err(|error| format!("refusing unsafe install directory: {error}"))?
        } else if user.is_different_from_effective_user() {
            return Err(format!(
                "refusing elevated update outside the invoking user's home: {}",
                path.display()
            ));
        } else {
            open_directory_descriptor(path, user.uid())
                .map_err(|error| format!("open install directory {}: {error}", path.display()))?
        };
        let pinned = Self {
            path: path.to_path_buf(),
            directory,
            owner_uid: user.uid(),
            owner_gid: user.gid(),
            chown_created_files: user.is_different_from_effective_user(),
        };
        pinned.ensure_public_binding()?;
        Ok(pinned)
    }

    #[cfg(test)]
    fn open_for_test(path: &Path) -> Result<Self, String> {
        // SAFETY: identity getters have no preconditions.
        let uid = unsafe { libc::geteuid() };
        let gid = unsafe { libc::getegid() };
        let directory = open_directory_descriptor(path, uid)
            .map_err(|error| format!("open test install directory: {error}"))?;
        Ok(Self {
            path: path.to_path_buf(),
            directory,
            owner_uid: uid,
            owner_gid: gid,
            chown_created_files: false,
        })
    }

    fn fd(&self) -> RawFd {
        self.directory.as_raw_fd()
    }

    fn display(&self, name: &str) -> String {
        self.path.join(name).display().to_string()
    }

    fn ensure_public_binding(&self) -> Result<(), String> {
        let pinned = fstat_descriptor(&self.directory)
            .map_err(|error| format!("inspect pinned install directory: {error}"))?;
        let current = lstat_path(&self.path).map_err(|error| {
            format!(
                "re-check install directory {}: {error}",
                self.path.display()
            )
        })?;
        if current.st_mode & libc::S_IFMT != libc::S_IFDIR
            || current.st_dev != pinned.st_dev
            || current.st_ino != pinned.st_ino
        {
            return Err(format!(
                "install directory {} changed after validation; refusing privileged update",
                self.path.display()
            ));
        }
        Ok(())
    }

    fn entry_stat(&self, name: &str) -> Result<Option<libc::stat>, String> {
        let name = safe_name(name)?;
        let mut stat = std::mem::MaybeUninit::<libc::stat>::uninit();
        // SAFETY: fd is a live directory, name is NUL-terminated, and stat is
        // writable. AT_SYMLINK_NOFOLLOW inspects the entry itself.
        let status = unsafe {
            libc::fstatat(
                self.fd(),
                name.as_ptr(),
                stat.as_mut_ptr(),
                libc::AT_SYMLINK_NOFOLLOW,
            )
        };
        if status == 0 {
            // SAFETY: fstatat succeeded and initialized stat.
            Ok(Some(unsafe { stat.assume_init() }))
        } else {
            let error = io::Error::last_os_error();
            if error.kind() == io::ErrorKind::NotFound {
                Ok(None)
            } else {
                Err(format!(
                    "inspect {}: {error}",
                    self.display(name.to_str().unwrap_or("?"))
                ))
            }
        }
    }

    fn entry_exists(&self, name: &str) -> Result<bool, String> {
        self.entry_stat(name).map(|stat| stat.is_some())
    }

    fn ensure_regular(&self, name: &str) -> Result<(), String> {
        match self.entry_stat(name)? {
            Some(stat) if stat.st_mode & libc::S_IFMT == libc::S_IFREG => Ok(()),
            Some(_) => Err(format!(
                "{} is not a regular file; refusing privileged update",
                self.display(name)
            )),
            None => Err(format!("expected file is missing: {}", self.display(name))),
        }
    }

    fn create_new(&self, name: &str, mode: libc::mode_t) -> Result<File, String> {
        let name_c = safe_name(name)?;
        // SAFETY: fd is live and name is NUL-terminated. O_EXCL and O_NOFOLLOW
        // prevent replacing or traversing an attacker-controlled entry.
        let fd = unsafe {
            libc::openat(
                self.fd(),
                name_c.as_ptr(),
                libc::O_WRONLY | libc::O_CREAT | libc::O_EXCL | libc::O_NOFOLLOW | libc::O_CLOEXEC,
                libc::c_uint::from(mode),
            )
        };
        if fd < 0 {
            Err(format!(
                "create {}: {}",
                self.display(name),
                io::Error::last_os_error()
            ))
        } else {
            // SAFETY: openat returned a new owned descriptor.
            Ok(unsafe { File::from_raw_fd(fd) })
        }
    }

    fn finalize_created(&self, file: &File, mode: u32) -> Result<(), String> {
        file.set_permissions(fs::Permissions::from_mode(mode))
            .and_then(|_| file.sync_all())
            .map_err(|error| format!("finalize updater file: {error}"))?;
        if self.chown_created_files {
            // SAFETY: the descriptor is live and UID/GID came from the
            // validated passwd entry.
            if unsafe { libc::fchown(file.as_raw_fd(), self.owner_uid, self.owner_gid) } != 0 {
                return Err(format!(
                    "assign updater file to invoking user: {}",
                    io::Error::last_os_error()
                ));
            }
            file.sync_all()
                .map_err(|error| format!("sync updater ownership metadata: {error}"))?;
        }
        Ok(())
    }

    fn rename(&self, old: &str, new: &str) -> Result<(), String> {
        let old_c = safe_name(old)?;
        let new_c = safe_name(new)?;
        // SAFETY: both names are NUL-terminated and remain relative to the
        // same pinned directory descriptor.
        if unsafe { libc::renameat(self.fd(), old_c.as_ptr(), self.fd(), new_c.as_ptr()) } == 0 {
            Ok(())
        } else {
            Err(format!(
                "rename {} to {}: {}",
                self.display(old),
                self.display(new),
                io::Error::last_os_error()
            ))
        }
    }

    fn unlink(&self, name: &str) -> Result<bool, String> {
        let name_c = safe_name(name)?;
        // SAFETY: name is NUL-terminated and relative to the pinned directory.
        if unsafe { libc::unlinkat(self.fd(), name_c.as_ptr(), 0) } == 0 {
            Ok(true)
        } else {
            let error = io::Error::last_os_error();
            if error.kind() == io::ErrorKind::NotFound {
                Ok(false)
            } else {
                Err(format!("remove {}: {error}", self.display(name)))
            }
        }
    }

    fn read_regular_limited(&self, name: &str, limit: u64) -> Result<Vec<u8>, String> {
        let name_c = safe_name(name)?;
        // SAFETY: fd is live and name is NUL-terminated. O_NOFOLLOW prevents
        // opening a symlink swapped in after the metadata check.
        let fd = unsafe {
            libc::openat(
                self.fd(),
                name_c.as_ptr(),
                libc::O_RDONLY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
            )
        };
        if fd < 0 {
            return Err(format!(
                "open {}: {}",
                self.display(name),
                io::Error::last_os_error()
            ));
        }
        // SAFETY: openat returned a new owned descriptor.
        let file = unsafe { File::from_raw_fd(fd) };
        let stat = fstat_descriptor(&file)
            .map_err(|error| format!("inspect {}: {error}", self.display(name)))?;
        if stat.st_mode & libc::S_IFMT != libc::S_IFREG || stat.st_size < 0 {
            return Err(format!("{} is not a regular file", self.display(name)));
        }
        if stat.st_size as u64 > limit {
            return Err(format!("{} exceeds its size limit", self.display(name)));
        }
        let mut bytes = Vec::new();
        file.take(limit + 1)
            .read_to_end(&mut bytes)
            .map_err(|error| format!("read {}: {error}", self.display(name)))?;
        if bytes.len() as u64 > limit {
            return Err(format!("{} exceeds its size limit", self.display(name)));
        }
        Ok(bytes)
    }

    fn sync(&self) -> Result<(), String> {
        self.directory
            .sync_all()
            .map_err(|error| format!("failed syncing directory {}: {error}", self.path.display()))
    }
}

fn safe_name(name: &str) -> Result<CString, String> {
    if name.is_empty() || name.as_bytes().contains(&b'/') {
        return Err(format!("unsafe updater filename {name:?}"));
    }
    CString::new(name).map_err(|_| format!("updater filename contains NUL: {name:?}"))
}

fn open_directory_descriptor(path: &Path, expected_uid: libc::uid_t) -> io::Result<File> {
    let path_c = CString::new(path.as_os_str().as_bytes())
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "directory path contains NUL"))?;
    // SAFETY: path is NUL-terminated. O_NOFOLLOW refuses a final symlink and
    // File takes ownership of a successful descriptor.
    let fd = unsafe {
        libc::open(
            path_c.as_ptr(),
            libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
        )
    };
    if fd < 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: open returned a new owned descriptor.
    let directory = unsafe { File::from_raw_fd(fd) };
    let stat = fstat_descriptor(&directory)?;
    if stat.st_mode & libc::S_IFMT != libc::S_IFDIR || stat.st_uid != expected_uid {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "install directory is not a directory owned by the invoking user",
        ));
    }
    Ok(directory)
}

fn fstat_descriptor(file: &File) -> io::Result<libc::stat> {
    let mut stat = std::mem::MaybeUninit::<libc::stat>::uninit();
    // SAFETY: stat is writable and fd is live.
    if unsafe { libc::fstat(file.as_raw_fd(), stat.as_mut_ptr()) } != 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: fstat succeeded and initialized stat.
    Ok(unsafe { stat.assume_init() })
}

fn lstat_path(path: &Path) -> io::Result<libc::stat> {
    let path_c = CString::new(path.as_os_str().as_bytes())
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "directory path contains NUL"))?;
    let mut stat = std::mem::MaybeUninit::<libc::stat>::uninit();
    // SAFETY: path is NUL-terminated and stat is writable. lstat inspects the
    // final path entry rather than following a replacement symlink.
    if unsafe { libc::lstat(path_c.as_ptr(), stat.as_mut_ptr()) } != 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: lstat succeeded and initialized stat.
    Ok(unsafe { stat.assume_init() })
}

#[derive(Debug)]
struct PairTransaction {
    directory: PinnedInstallDir,
    committed: bool,
}

impl PairTransaction {
    fn begin(directory: PinnedInstallDir, pair: &ExtractedPair) -> Result<Self, String> {
        directory.ensure_public_binding()?;
        for name in ["nd300", "speedqx"] {
            directory.ensure_regular(name).map_err(|error| {
                format!("refusing update because expected installed binary is unsafe: {error}")
            })?;
        }
        ensure_no_transaction_files(&directory)?;
        if let Err(error) = copy_for_transaction(&directory, &pair.nd300, NEW_ND300) {
            cleanup_pretransaction_files(&directory);
            return Err(error);
        }
        if let Err(error) = copy_for_transaction(&directory, &pair.speedqx, NEW_SPEEDQX) {
            cleanup_pretransaction_files(&directory);
            return Err(error);
        }
        let marker_result = (|| {
            let mut marker_file = directory.create_new(TRANSACTION_MARKER, 0o600)?;
            marker_file
                .write_all(b"nd300-two-binary-update-v1\n")
                .map_err(|error| format!("write transaction marker: {error}"))?;
            directory.finalize_created(&marker_file, 0o600)
        })();
        if let Err(error) = marker_result {
            cleanup_pretransaction_files(&directory);
            return Err(format!("failed to persist transaction marker: {error}"));
        }
        if let Err(error) = directory.sync() {
            let cleanup = rollback_transaction(&directory);
            return match cleanup {
                Ok(()) => Err(error),
                Err(cleanup_error) => Err(format!(
                    "{error}; pre-transaction cleanup also failed: {cleanup_error}"
                )),
            };
        }

        let transaction = Self {
            directory,
            committed: false,
        };
        if let Err(error) = transaction.swap_files() {
            let rollback = transaction.rollback_internal();
            return match rollback {
                Ok(()) => Err(error),
                Err(rollback_error) => {
                    Err(format!("{error}; rollback also failed: {rollback_error}"))
                }
            };
        }
        Ok(transaction)
    }

    fn swap_files(&self) -> Result<(), String> {
        // Replace one binary at a time and keep the other runnable. `speedqx`
        // exposes the same update action, so a crash between renames still
        // leaves a command capable of recovering the persisted marker.
        self.directory.rename("speedqx", BACKUP_SPEEDQX)?;
        self.directory.rename(NEW_SPEEDQX, "speedqx")?;
        self.directory.rename("nd300", BACKUP_ND300)?;
        self.directory.rename(NEW_ND300, "nd300")?;
        self.directory.sync()
    }

    fn commit(&mut self) -> Result<Option<String>, String> {
        let mut marker_file = self.directory.create_new(COMMIT_MARKER_STAGE, 0o600)?;
        marker_file
            .write_all(b"nd300-two-binary-update-v1:committed\n")
            .map_err(|error| format!("write committed transaction marker: {error}"))?;
        self.directory.finalize_created(&marker_file, 0o600)?;
        self.directory
            .rename(COMMIT_MARKER_STAGE, TRANSACTION_MARKER)?;
        self.directory.sync()?;
        // From this point forward recovery must keep the verified new pair. Mark
        // Drop as committed before best-effort backup cleanup so a cleanup error
        // cannot accidentally roll back only one binary.
        self.committed = true;
        match cleanup_committed_transaction(&self.directory) {
            Ok(()) => Ok(None),
            Err(error) => Ok(Some(format!(
                "the new verified binary pair is active, but committed updater backups could not be fully cleaned: {error}. The committed marker will retry cleanup on the next update"
            ))),
        }
    }

    fn rollback(&mut self) -> Result<(), String> {
        let result = self.rollback_internal();
        if result.is_ok() {
            self.committed = true;
        }
        result
    }

    fn rollback_internal(&self) -> Result<(), String> {
        rollback_transaction(&self.directory)
    }
}

fn cleanup_pretransaction_files(directory: &PinnedInstallDir) {
    for name in [
        NEW_ND300,
        NEW_SPEEDQX,
        TRANSACTION_MARKER,
        COMMIT_MARKER_STAGE,
    ] {
        let _ = directory.unlink(name);
    }
}

impl Drop for PairTransaction {
    fn drop(&mut self) {
        if !self.committed {
            let _ = self.rollback_internal();
        }
    }
}

fn copy_for_transaction(
    directory: &PinnedInstallDir,
    source: &Path,
    destination: &str,
) -> Result<(), String> {
    let mut input = File::open(source)
        .map_err(|error| format!("failed opening {}: {error}", source.display()))?;
    let mut output = directory.create_new(destination, 0o700)?;
    io::copy(&mut input, &mut output)
        .map_err(|error| format!("failed copying {}: {error}", directory.display(destination)))?;
    directory.finalize_created(&output, 0o755)
}

fn ensure_no_transaction_files(directory: &PinnedInstallDir) -> Result<(), String> {
    for name in [
        TRANSACTION_MARKER,
        COMMIT_MARKER_STAGE,
        NEW_ND300,
        NEW_SPEEDQX,
        BACKUP_ND300,
        BACKUP_SPEEDQX,
    ] {
        if directory.entry_exists(name)? {
            return Err(format!(
                "unexpected updater transaction file {}; refusing to overwrite it",
                directory.display(name)
            ));
        }
    }
    Ok(())
}

fn recover_interrupted_transaction(directory: &PinnedInstallDir) -> Result<(), String> {
    if !directory.entry_exists(TRANSACTION_MARKER)? {
        return ensure_no_transaction_files(directory);
    }
    let marker_bytes = directory
        .read_regular_limited(TRANSACTION_MARKER, 1024)
        .map_err(|error| format!("could not read transaction marker: {error}"))?;
    if marker_bytes == b"nd300-two-binary-update-v1:committed\n" {
        return cleanup_committed_transaction(directory);
    }
    if marker_bytes != b"nd300-two-binary-update-v1\n" {
        return Err(format!(
            "unrecognized transaction marker {}; refusing recovery",
            directory.display(TRANSACTION_MARKER)
        ));
    }
    rollback_transaction(directory).map_err(|error| {
        format!(
            "could not recover interrupted update in {}: {error}",
            directory.path.display()
        )
    })
}

fn cleanup_committed_transaction(directory: &PinnedInstallDir) -> Result<(), String> {
    let mut failures = Vec::new();
    for name in [
        BACKUP_ND300,
        BACKUP_SPEEDQX,
        NEW_ND300,
        NEW_SPEEDQX,
        COMMIT_MARKER_STAGE,
    ] {
        match directory.entry_exists(name) {
            Ok(false) => {}
            Ok(true) => {
                if let Err(error) = directory
                    .ensure_regular(name)
                    .and_then(|_| directory.unlink(name).map(|_| ()))
                {
                    failures.push(error);
                }
            }
            Err(error) => failures.push(error),
        }
    }
    if failures.is_empty() {
        directory.ensure_regular(TRANSACTION_MARKER)?;
        directory.unlink(TRANSACTION_MARKER)?;
        directory.sync()
    } else {
        Err(failures.join("; "))
    }
}

fn rollback_transaction(directory: &PinnedInstallDir) -> Result<(), String> {
    let mut failures = Vec::new();
    for (binary, backup) in [("nd300", BACKUP_ND300), ("speedqx", BACKUP_SPEEDQX)] {
        match directory.entry_exists(backup) {
            Ok(false) => {}
            Ok(true) => {
                if let Err(error) = directory.ensure_regular(backup) {
                    failures.push(error);
                    continue;
                }
                match directory.entry_exists(binary) {
                    Ok(true) => {
                        if let Err(error) = directory
                            .ensure_regular(binary)
                            .and_then(|_| directory.unlink(binary).map(|_| ()))
                        {
                            failures.push(error);
                            continue;
                        }
                    }
                    Ok(false) => {}
                    Err(error) => {
                        failures.push(error);
                        continue;
                    }
                }
                if let Err(error) = directory.rename(backup, binary) {
                    failures.push(error);
                }
            }
            Err(error) => failures.push(error),
        }
    }
    for name in [NEW_ND300, NEW_SPEEDQX, COMMIT_MARKER_STAGE] {
        match directory.entry_exists(name) {
            Ok(false) => {}
            Ok(true) => {
                if let Err(error) = directory
                    .ensure_regular(name)
                    .and_then(|_| directory.unlink(name).map(|_| ()))
                {
                    failures.push(error);
                }
            }
            Err(error) => failures.push(error),
        }
    }
    if failures.is_empty() {
        if directory.entry_exists(TRANSACTION_MARKER)? {
            directory.ensure_regular(TRANSACTION_MARKER)?;
            directory.unlink(TRANSACTION_MARKER)?;
        }
        directory.sync()
    } else {
        Err(failures.join("; "))
    }
}

struct SecureTempDir {
    path: PathBuf,
}

impl SecureTempDir {
    fn new() -> Result<Self, String> {
        let base = std::env::temp_dir();
        let seed = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        for attempt in 0..32_u32 {
            let path = base.join(format!(
                "nd300-update-{}-{seed}-{attempt}",
                std::process::id()
            ));
            match fs::DirBuilder::new().mode(0o700).create(&path) {
                Ok(()) => return Ok(Self { path }),
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(error) => return Err(format!("failed creating temp directory: {error}")),
            }
        }
        Err("could not create a unique updater temp directory".to_string())
    }

    fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for SecureTempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_test_archive(path: &Path, target: &str, duplicate_nd300: bool) {
        let file = File::create(path).unwrap();
        let encoder = xz2::write::XzEncoder::new(file, 6);
        let mut builder = tar::Builder::new(encoder);
        let root = format!("nd300-{target}");

        for (name, bytes, mode) in [
            ("README.md", b"docs".as_slice(), 0o644),
            ("nd300", b"nd300-bytes".as_slice(), 0o755),
            ("speedqx", b"speedqx-bytes".as_slice(), 0o755),
        ] {
            let mut header = tar::Header::new_gnu();
            header.set_size(bytes.len() as u64);
            header.set_mode(mode);
            header.set_cksum();
            builder
                .append_data(&mut header, format!("{root}/{name}"), bytes)
                .unwrap();
        }
        if duplicate_nd300 {
            let bytes = b"duplicate";
            let mut header = tar::Header::new_gnu();
            header.set_size(bytes.len() as u64);
            header.set_mode(0o755);
            header.set_cksum();
            builder
                .append_data(&mut header, format!("{root}/nd300"), bytes.as_slice())
                .unwrap();
        }
        let encoder = builder.into_inner().unwrap();
        encoder.finish().unwrap();
    }

    fn write_link_test_archive(path: &Path, target: &str, entry_type: tar::EntryType) {
        let file = File::create(path).unwrap();
        let encoder = xz2::write::XzEncoder::new(file, 6);
        let mut builder = tar::Builder::new(encoder);
        let mut header = tar::Header::new_gnu();
        header.set_entry_type(entry_type);
        header.set_size(0);
        header.set_mode(0o755);
        header.set_link_name("../../outside").unwrap();
        header.set_cksum();
        builder
            .append_data(&mut header, format!("nd300-{target}/nd300"), io::empty())
            .unwrap();
        let encoder = builder.into_inner().unwrap();
        encoder.finish().unwrap();
    }

    fn temp_dir(label: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "nd300-unix-install-test-{label}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        ));
        fs::create_dir(&path).unwrap();
        path.canonicalize().unwrap()
    }

    fn pinned(path: &Path) -> PinnedInstallDir {
        PinnedInstallDir::open_for_test(path).unwrap()
    }

    #[test]
    fn classifies_known_and_refused_origins() {
        let cargo = Path::new("/Users/alice/.cargo/bin");
        assert_eq!(
            classify_origin_path(
                Path::new("/Users/alice/.cargo/bin/nd300"),
                cargo,
                true,
                false,
            ),
            UnixOriginKind::Cargo
        );
        assert_eq!(
            classify_origin_path(
                Path::new("/Users/alice/.cargo/bin/nd300"),
                cargo,
                false,
                true,
            ),
            UnixOriginKind::ManagedArchive
        );
        assert_eq!(
            classify_origin_path(
                Path::new("/Users/alice/.cargo/bin/nd300"),
                cargo,
                true,
                true,
            ),
            UnixOriginKind::Unknown
        );
        assert_eq!(
            classify_origin_path(Path::new("/opt/homebrew/bin/nd300"), cargo, false, true),
            UnixOriginKind::PackageManager
        );
        assert_eq!(
            classify_origin_path(Path::new("/repo/target/release/nd300"), cargo, false, true),
            UnixOriginKind::LocalBuild
        );
        assert_eq!(
            classify_origin_path(Path::new("/usr/local/bin/nd300"), cargo, false, true),
            UnixOriginKind::Unknown
        );
        assert_eq!(
            classify_origin_path(
                Path::new("/Users/alice/.cargo/bin/nd300"),
                cargo,
                false,
                false,
            ),
            UnixOriginKind::Unknown
        );
    }

    #[test]
    fn elevated_cargo_selection_never_falls_back_to_root_path() {
        let missing = Path::new("/definitely/missing/invoking-user-cargo");
        assert_eq!(select_cargo_executable(missing, true), None);
        assert_eq!(
            select_cargo_executable(missing, false),
            Some(PathBuf::from("cargo"))
        );
        assert_eq!(
            cargo_install_args("3.6.0"),
            vec![
                "install",
                "nd300",
                "--version",
                "3.6.0",
                "--force",
                "--locked"
            ]
        );
    }

    #[test]
    fn symlink_does_not_make_an_unknown_update_target_managed() {
        let cargo = Path::new("/Users/alice/.cargo/bin");
        let target = Path::new("/Users/alice/.cargo/bin/nd300");
        assert_eq!(
            effective_update_kind(UnixOriginKind::Symlink, target, cargo, false, false),
            UnixOriginKind::Unknown
        );
        assert_eq!(
            effective_update_kind(UnixOriginKind::Symlink, target, cargo, true, false),
            UnixOriginKind::Cargo
        );
        assert_eq!(
            effective_update_kind(UnixOriginKind::Symlink, target, cargo, false, true),
            UnixOriginKind::ManagedArchive
        );
    }

    fn valid_receipt(prefix: &str) -> Vec<u8> {
        serde_json::to_vec(&serde_json::json!({
            "binaries": ["nd300", "speedqx"],
            "install_layout": "cargo-home",
            "install_prefix": prefix,
            "provider": {"source": "cargo-dist", "version": "0.31.0"},
            "source": {
                "app_name": "nd300",
                "name": "qube-network-diagnostics",
                "owner": "QubeTX",
                "release_type": "github"
            },
            "version": "3.5.2"
        }))
        .unwrap()
    }

    #[test]
    fn managed_archive_requires_exact_validated_cargo_dist_receipt() {
        let cargo_home = Path::new("/Users/alice/.cargo");
        assert!(validate_cargo_dist_receipt(
            &valid_receipt("/Users/alice/.cargo"),
            cargo_home
        ));
        assert!(!validate_cargo_dist_receipt(b"not-json", cargo_home));

        let mut wrong_prefix: serde_json::Value =
            serde_json::from_slice(&valid_receipt("/var/root/.cargo")).unwrap();
        assert!(!validate_cargo_dist_receipt(
            &serde_json::to_vec(&wrong_prefix).unwrap(),
            cargo_home
        ));
        wrong_prefix["install_prefix"] = serde_json::json!("/Users/alice/.cargo");
        wrong_prefix["provider"]["source"] = serde_json::json!("manual-copy");
        assert!(!validate_cargo_dist_receipt(
            &serde_json::to_vec(&wrong_prefix).unwrap(),
            cargo_home
        ));

        let mut wrong_bins: serde_json::Value =
            serde_json::from_slice(&valid_receipt("/Users/alice/.cargo")).unwrap();
        wrong_bins["binaries"] = serde_json::json!(["nd300", "speedqx", "cargo"]);
        assert!(!validate_cargo_dist_receipt(
            &serde_json::to_vec(&wrong_bins).unwrap(),
            cargo_home
        ));
    }

    #[test]
    fn validated_receipt_cleanup_preserves_unknown_user_files() {
        let dir = temp_dir("receipt-cleanup");
        let cargo_home = dir.join(".cargo");
        fs::create_dir(&cargo_home).unwrap();
        let receipt_dir = dir.join(".config").join("nd300");
        fs::create_dir_all(&receipt_dir).unwrap();
        let receipt_path = receipt_dir.join("nd300-receipt.json");
        let bytes = valid_receipt(cargo_home.to_str().unwrap());
        fs::write(&receipt_path, &bytes).unwrap();
        fs::write(receipt_dir.join("user-note.txt"), b"keep me").unwrap();
        let captured = ValidatedReceipt {
            path: receipt_path.clone(),
            bytes,
        };

        assert!(remove_captured_receipt(&captured, &cargo_home).unwrap());
        assert!(!receipt_path.exists());
        assert_eq!(
            fs::read(receipt_dir.join("user-note.txt")).unwrap(),
            b"keep me"
        );
        assert!(receipt_dir.exists());
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn archive_paths_reject_traversal_and_absolute_paths() {
        assert!(validate_archive_path(Path::new("nd300-target/nd300")).is_ok());
        assert!(validate_archive_path(Path::new("../nd300")).is_err());
        assert!(validate_archive_path(Path::new("root/../../nd300")).is_err());
        assert!(validate_archive_path(Path::new("/tmp/nd300")).is_err());
    }

    #[test]
    fn archive_extraction_allows_only_the_expected_binary_pair() {
        let dir = temp_dir("extract");
        let archive = dir.join("fixture.tar.xz");
        let output = dir.join("output");
        fs::create_dir(&output).unwrap();
        write_test_archive(&archive, "aarch64-apple-darwin", false);

        let pair = extract_archive_allowlisted(&archive, &output, "aarch64-apple-darwin").unwrap();
        assert_eq!(fs::read(pair.nd300).unwrap(), b"nd300-bytes");
        assert_eq!(fs::read(pair.speedqx).unwrap(), b"speedqx-bytes");
        assert!(!output.join("README.md").exists());
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn archive_extraction_rejects_duplicate_binaries() {
        let dir = temp_dir("duplicate");
        let archive = dir.join("fixture.tar.xz");
        let output = dir.join("output");
        fs::create_dir(&output).unwrap();
        write_test_archive(&archive, "x86_64-unknown-linux-gnu", true);
        let error =
            extract_archive_allowlisted(&archive, &output, "x86_64-unknown-linux-gnu").unwrap_err();
        assert!(error.contains("duplicate nd300"));
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn archive_extraction_rejects_symlinks_and_hardlinks() {
        for (label, entry_type) in [
            ("symlink", tar::EntryType::symlink()),
            ("hardlink", tar::EntryType::hard_link()),
        ] {
            let dir = temp_dir(label);
            let archive = dir.join("fixture.tar.xz");
            let output = dir.join("output");
            fs::create_dir(&output).unwrap();
            write_link_test_archive(&archive, "aarch64-apple-darwin", entry_type);
            let error =
                extract_archive_allowlisted(&archive, &output, "aarch64-apple-darwin").unwrap_err();
            assert!(error.contains("forbidden link entry"));
            fs::remove_dir_all(dir).unwrap();
        }
    }

    #[test]
    fn sidecar_and_checksum_are_fail_closed() {
        let hash = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";
        let sidecar = format!("{hash}  *archive.tar.xz\n");
        assert_eq!(
            parse_sha256_sidecar(sidecar.as_bytes(), "archive.tar.xz").as_deref(),
            Some(hash)
        );
        assert!(parse_sha256_sidecar(sidecar.as_bytes(), "other.tar.xz").is_none());
        assert!(parse_sha256_sidecar(hash.as_bytes(), "archive.tar.xz").is_none());
        assert!(checksum_verdict(hash, hash).is_ok());
        assert!(checksum_verdict("deadbeef", hash).is_err());
        assert!(parse_sha256_sidecar(b"not-a-hash archive", "archive").is_none());
    }

    #[test]
    fn version_matching_rejects_wrong_or_empty_versions() {
        assert!(versions_match("3.6.0", "3.6.0"));
        assert!(!versions_match("3.6.0+sha.abc", "3.6.0"));
        assert!(!versions_match("3.6.0-rc.1", "3.6.0"));
        assert!(!versions_match("3.5.2", "3.6.0"));
        assert!(!versions_match("", "3.6.0"));
    }

    #[test]
    fn release_version_validation_rejects_url_and_tag_injection() {
        assert!(validate_release_version("3.6.0").is_ok());
        assert!(validate_release_version("3.6").is_err());
        assert!(validate_release_version("3.6.0/../../evil").is_err());
        assert!(validate_release_version("3.6.0-rc.1").is_err());
        assert!(validate_release_version("v3.6.0").is_err());
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn signature_details_require_exact_identifier_and_team_fields() {
        let details = "Identifier=com.qubetx.nd300\nAuthority=Developer ID Application: Qube LLC (M9D5379H93)\nTeamIdentifier=M9D5379H93\nTimestamp=Jul 17, 2026\nflags=0x10000(runtime)\n";
        assert_eq!(
            signing_field(details, "Identifier"),
            Some("com.qubetx.nd300")
        );
        assert_eq!(
            signing_field(details, "TeamIdentifier"),
            Some(MACOS_TEAM_ID)
        );
        assert_eq!(
            signing_field(details, "Authority"),
            Some("Developer ID Application: Qube LLC (M9D5379H93)")
        );
        assert!(signing_field(details, "Timestamp").is_some());
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn embedded_leaf_fingerprint_is_pinned_not_subject_only() {
        let fingerprint = sha1_fingerprint(b"fixture leaf certificate");
        assert_eq!(fingerprint.len(), 40);
        assert!(fingerprint_verdict(MACOS_LEAF_SHA1, MACOS_LEAF_SHA1).is_ok());
        let error = fingerprint_verdict(&fingerprint, MACOS_LEAF_SHA1).unwrap_err();
        assert!(error.contains("leaf certificate mismatch"));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn gatekeeper_install_policy_distinguishes_notarized_and_unnotarized_clis() {
        // Captured on macOS 26 from the Accepted-notarized TR-300 v4.0.1 CLI.
        let notarized = "accepted\nsource=Notarized Developer ID\n";
        assert!(gatekeeper_install_verdict(true, notarized).is_ok());

        // Captured from the same local Developer ID identity, identifier,
        // hardened runtime and timestamp, but without Apple notarization.
        let unnotarized = "rejected\nsource=Unnotarized Developer ID\n";
        assert!(gatekeeper_install_verdict(false, unnotarized).is_err());
        assert!(gatekeeper_install_verdict(true, unnotarized).is_err());

        // Execute policy is not evidence of notarization for a bare CLI and
        // must never regain the old success exception.
        let bare_cli = "rejected (the code is valid but does not seem to be an app)";
        assert!(gatekeeper_install_verdict(false, bare_cli).is_err());
        assert!(gatekeeper_install_verdict(false, "").is_err());
    }

    #[test]
    fn interrupted_transaction_rolls_back_both_binaries() {
        let dir = temp_dir("rollback");
        fs::write(dir.join("nd300"), b"old-nd300").unwrap();
        fs::write(dir.join("speedqx"), b"old-speedqx").unwrap();
        fs::write(dir.join(BACKUP_ND300), b"older-nd300").unwrap();
        fs::write(dir.join(BACKUP_SPEEDQX), b"older-speedqx").unwrap();
        fs::write(
            dir.join(TRANSACTION_MARKER),
            b"nd300-two-binary-update-v1\n",
        )
        .unwrap();
        fs::write(dir.join(NEW_ND300), b"new-nd300").unwrap();
        fs::write(dir.join(NEW_SPEEDQX), b"new-speedqx").unwrap();

        recover_interrupted_transaction(&pinned(&dir)).unwrap();
        assert_eq!(fs::read(dir.join("nd300")).unwrap(), b"older-nd300");
        assert_eq!(fs::read(dir.join("speedqx")).unwrap(), b"older-speedqx");
        assert!(!dir.join(TRANSACTION_MARKER).exists());
        assert!(!dir.join(NEW_ND300).exists());
        assert!(!dir.join(NEW_SPEEDQX).exists());
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn interrupted_commit_marker_staging_rolls_back_the_pair() {
        let dir = temp_dir("commit-marker-staging");
        fs::write(dir.join("nd300"), b"new-nd300").unwrap();
        fs::write(dir.join("speedqx"), b"new-speedqx").unwrap();
        fs::write(dir.join(BACKUP_ND300), b"old-nd300").unwrap();
        fs::write(dir.join(BACKUP_SPEEDQX), b"old-speedqx").unwrap();
        fs::write(
            dir.join(TRANSACTION_MARKER),
            b"nd300-two-binary-update-v1\n",
        )
        .unwrap();
        fs::write(
            dir.join(COMMIT_MARKER_STAGE),
            b"nd300-two-binary-update-v1:committed\n",
        )
        .unwrap();

        recover_interrupted_transaction(&pinned(&dir)).unwrap();
        assert_eq!(fs::read(dir.join("nd300")).unwrap(), b"old-nd300");
        assert_eq!(fs::read(dir.join("speedqx")).unwrap(), b"old-speedqx");
        assert!(!dir.join(COMMIT_MARKER_STAGE).exists());
        assert!(!dir.join(TRANSACTION_MARKER).exists());
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn every_partial_swap_transition_recovers_the_old_pair() {
        for completed_steps in 1..=4 {
            let dir = temp_dir(&format!("partial-swap-{completed_steps}"));
            fs::write(dir.join("nd300"), b"old-nd300").unwrap();
            fs::write(dir.join("speedqx"), b"old-speedqx").unwrap();
            fs::write(dir.join(NEW_ND300), b"new-nd300").unwrap();
            fs::write(dir.join(NEW_SPEEDQX), b"new-speedqx").unwrap();
            fs::write(
                dir.join(TRANSACTION_MARKER),
                b"nd300-two-binary-update-v1\n",
            )
            .unwrap();

            let transitions = [
                ("speedqx", BACKUP_SPEEDQX),
                (NEW_SPEEDQX, "speedqx"),
                ("nd300", BACKUP_ND300),
                (NEW_ND300, "nd300"),
            ];
            for (from, to) in transitions.iter().take(completed_steps) {
                fs::rename(dir.join(from), dir.join(to)).unwrap();
            }

            recover_interrupted_transaction(&pinned(&dir)).unwrap();
            assert_eq!(fs::read(dir.join("nd300")).unwrap(), b"old-nd300");
            assert_eq!(fs::read(dir.join("speedqx")).unwrap(), b"old-speedqx");
            assert!(!dir.join(TRANSACTION_MARKER).exists());
            fs::remove_dir_all(dir).unwrap();
        }
    }

    #[test]
    fn active_transaction_can_roll_back_after_partial_verification_failure() {
        let dir = temp_dir("active-rollback");
        let staged = temp_dir("active-staged");
        fs::write(dir.join("nd300"), b"old-nd300").unwrap();
        fs::write(dir.join("speedqx"), b"old-speedqx").unwrap();
        fs::write(staged.join("nd300"), b"new-nd300").unwrap();
        fs::write(staged.join("speedqx"), b"new-speedqx").unwrap();
        let pair = ExtractedPair {
            nd300: staged.join("nd300"),
            speedqx: staged.join("speedqx"),
        };
        let mut transaction = PairTransaction::begin(pinned(&dir), &pair).unwrap();
        assert_eq!(fs::read(dir.join("nd300")).unwrap(), b"new-nd300");
        assert_eq!(fs::read(dir.join("speedqx")).unwrap(), b"new-speedqx");

        transaction.rollback().unwrap();
        assert_eq!(fs::read(dir.join("nd300")).unwrap(), b"old-nd300");
        assert_eq!(fs::read(dir.join("speedqx")).unwrap(), b"old-speedqx");
        fs::remove_dir_all(dir).unwrap();
        fs::remove_dir_all(staged).unwrap();
    }

    #[test]
    fn committed_recovery_keeps_new_pair_and_only_cleans_backups() {
        let dir = temp_dir("committed-recovery");
        fs::write(dir.join("nd300"), b"new-nd300").unwrap();
        fs::write(dir.join("speedqx"), b"new-speedqx").unwrap();
        fs::write(dir.join(BACKUP_ND300), b"old-nd300").unwrap();
        fs::write(dir.join(BACKUP_SPEEDQX), b"old-speedqx").unwrap();
        fs::write(
            dir.join(TRANSACTION_MARKER),
            b"nd300-two-binary-update-v1:committed\n",
        )
        .unwrap();

        recover_interrupted_transaction(&pinned(&dir)).unwrap();
        assert_eq!(fs::read(dir.join("nd300")).unwrap(), b"new-nd300");
        assert_eq!(fs::read(dir.join("speedqx")).unwrap(), b"new-speedqx");
        assert!(!dir.join(BACKUP_ND300).exists());
        assert!(!dir.join(BACKUP_SPEEDQX).exists());
        assert!(!dir.join(TRANSACTION_MARKER).exists());
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn committed_backup_cleanup_failure_keeps_verified_new_pair() {
        let dir = temp_dir("committed-cleanup-warning");
        let staged = temp_dir("committed-cleanup-staged");
        fs::write(dir.join("nd300"), b"old-nd300").unwrap();
        fs::write(dir.join("speedqx"), b"old-speedqx").unwrap();
        fs::write(staged.join("nd300"), b"new-nd300").unwrap();
        fs::write(staged.join("speedqx"), b"new-speedqx").unwrap();
        let pair = ExtractedPair {
            nd300: staged.join("nd300"),
            speedqx: staged.join("speedqx"),
        };
        let mut transaction = PairTransaction::begin(pinned(&dir), &pair).unwrap();
        fs::remove_file(dir.join(BACKUP_SPEEDQX)).unwrap();
        fs::create_dir(dir.join(BACKUP_SPEEDQX)).unwrap();

        let warning = transaction.commit().unwrap().unwrap();
        assert!(warning.contains("new verified binary pair is active"));
        assert_eq!(fs::read(dir.join("nd300")).unwrap(), b"new-nd300");
        assert_eq!(fs::read(dir.join("speedqx")).unwrap(), b"new-speedqx");
        assert_eq!(
            fs::read(dir.join(TRANSACTION_MARKER)).unwrap(),
            b"nd300-two-binary-update-v1:committed\n"
        );
        fs::remove_dir_all(dir).unwrap();
        fs::remove_dir_all(staged).unwrap();
    }

    #[test]
    fn orphan_transaction_files_are_refused_without_marker() {
        let dir = temp_dir("orphan");
        fs::write(dir.join(BACKUP_ND300), b"do-not-overwrite").unwrap();
        let error = recover_interrupted_transaction(&pinned(&dir)).unwrap_err();
        assert!(error.contains("refusing to overwrite"));
        assert_eq!(
            fs::read(dir.join(BACKUP_ND300)).unwrap(),
            b"do-not-overwrite"
        );
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn transaction_refuses_installed_binary_symlinks() {
        use std::os::unix::fs::symlink;

        let dir = temp_dir("binary-symlink");
        let staged = temp_dir("binary-symlink-staged");
        let victim = dir.join("victim");
        fs::write(&victim, b"outside target").unwrap();
        symlink(&victim, dir.join("nd300")).unwrap();
        fs::write(dir.join("speedqx"), b"old-speedqx").unwrap();
        fs::write(staged.join("nd300"), b"new-nd300").unwrap();
        fs::write(staged.join("speedqx"), b"new-speedqx").unwrap();
        let pair = ExtractedPair {
            nd300: staged.join("nd300"),
            speedqx: staged.join("speedqx"),
        };

        let error = PairTransaction::begin(pinned(&dir), &pair).unwrap_err();
        assert!(error.contains("not a regular file"));
        assert_eq!(fs::read(&victim).unwrap(), b"outside target");
        assert!(!dir.join(NEW_ND300).exists());
        fs::remove_dir_all(dir).unwrap();
        fs::remove_dir_all(staged).unwrap();
    }

    #[test]
    fn recovery_refuses_a_symlink_marker_without_reading_its_target() {
        use std::os::unix::fs::symlink;

        let dir = temp_dir("marker-symlink");
        let victim = dir.join("victim");
        fs::write(&victim, b"nd300-two-binary-update-v1\n").unwrap();
        symlink(&victim, dir.join(TRANSACTION_MARKER)).unwrap();

        let error = recover_interrupted_transaction(&pinned(&dir)).unwrap_err();
        assert!(error.contains("transaction marker"));
        assert_eq!(fs::read(&victim).unwrap(), b"nd300-two-binary-update-v1\n");
        assert!(fs::symlink_metadata(dir.join(TRANSACTION_MARKER))
            .unwrap()
            .file_type()
            .is_symlink());
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn pinned_directory_operations_cannot_be_redirected_by_path_replacement() {
        use std::os::unix::fs::symlink;

        let root = temp_dir("pinned-path-replacement");
        let public = root.join("bin");
        let moved = root.join("original-bin");
        let victim = root.join("victim");
        fs::create_dir(&public).unwrap();
        fs::create_dir(&victim).unwrap();
        let pinned = pinned(&public);

        fs::rename(&public, &moved).unwrap();
        symlink(&victim, &public).unwrap();
        let mut proof = pinned.create_new("proof", 0o600).unwrap();
        proof.write_all(b"pinned\n").unwrap();
        pinned.finalize_created(&proof, 0o600).unwrap();

        assert_eq!(fs::read(moved.join("proof")).unwrap(), b"pinned\n");
        assert!(!victim.join("proof").exists());
        assert!(pinned.ensure_public_binding().is_err());
        fs::remove_dir_all(root).unwrap();
    }
}
