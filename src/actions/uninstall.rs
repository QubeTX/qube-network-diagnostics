use crate::config::{Config, OutputFormat};
use crate::render::color;
use std::path::{Path, PathBuf};

use super::{fail_icon, is_interactive, prompt_yes_no, success_icon};

/// Binaries that are part of this package (installed together by cargo-dist).
/// This is the deletion allowlist: `migrate-cleanup` (src/actions/migrate.rs) only
/// ever deletes files whose stem is in this list, so it can never touch
/// `cargo.exe`, `rustup.exe`, or any other binary that shares a directory.
pub(crate) const OUR_BINARIES: &[&str] = &["nd300", "speedqx"];

/// Tracks what we cleaned up for reporting.
pub(crate) struct CleanupReport {
    /// True only when the binary is actually gone now (Unix `remove_file`, or a
    /// non-running sibling delete). On Windows the *running* exe can't be deleted
    /// in place, so this stays false even on a successful uninstall — see
    /// `binary_removal_scheduled`.
    pub(crate) binary_removed: bool,
    /// Windows-only: the running exe couldn't be removed now, but a background
    /// `cmd /C … del` was successfully spawned to delete it once this process
    /// exits. Kept distinct from `binary_removed` so the updater's shadow-cleanup
    /// guard can tell "already gone" from "scheduled for removal on exit" and not
    /// be silently defeated by an optimistic "removed".
    pub(crate) binary_removal_scheduled: bool,
    /// Unix symlink invocation: only the link was removed; the underlying
    /// package-manager/Cargo/archive target remains installed.
    pub(crate) target_retained: bool,
    pub(crate) sibling_removed: bool,
    pub(crate) receipt_removed: bool,
    pub(crate) path_cleaned: bool,
    pub(crate) notes: Vec<String>,
}

pub async fn run(config: &Config) -> i32 {
    let exe_path = match std::env::current_exe() {
        Ok(p) => p,
        Err(e) => {
            if config.format == OutputFormat::Json {
                let output = serde_json::json!({
                    "action": "uninstall",
                    "success": false,
                    "message": format!("Could not determine binary location: {}", e),
                });
                println!(
                    "{}",
                    serde_json::to_string_pretty(&output).unwrap_or_else(|_| "{}".to_string())
                );
            } else {
                println!(
                    "  {} {}",
                    color::red(fail_icon(config), config),
                    color::red(
                        &format!("Could not determine binary location: {}", e),
                        config
                    ),
                );
            }
            return 2;
        }
    };

    // Resolve symlinks to get the real path
    let real_path = match exe_path.canonicalize() {
        Ok(p) => p,
        Err(_) => exe_path.clone(),
    };

    if config.format == OutputFormat::Json {
        return run_json(&real_path, config).await;
    }

    println!();
    #[cfg(unix)]
    print_unix_uninstall_preview(&real_path, config);
    #[cfg(windows)]
    println!(
        "  This will remove nd300 and speedqx from: {}",
        color::cyan(
            &real_path
                .parent()
                .unwrap_or(&real_path)
                .display()
                .to_string(),
            config
        ),
    );

    // Show what we'll clean up
    let receipt_dir = get_receipt_dir();
    #[cfg(unix)]
    let show_receipt = unix_uninstall_cleans_receipt();
    #[cfg(windows)]
    let show_receipt = true;
    if let Some(ref dir) = receipt_dir {
        if show_receipt && dir.exists() {
            println!(
                "  Config/receipt directory:    {}",
                color::cyan(&dir.display().to_string(), config),
            );
        }
    }

    #[cfg(windows)]
    {
        let bin_dir = real_path.parent().map(|p| p.to_path_buf());
        if let Some(ref dir) = bin_dir {
            if is_sole_package_in_dir(dir) {
                println!(
                    "  PATH entry to clean:        {}",
                    color::cyan(&dir.display().to_string(), config),
                );
            }
        }
    }

    println!();

    if is_interactive(config) {
        #[cfg(unix)]
        let prompt = "  Proceed with this origin-aware uninstall? (y/N): ";
        #[cfg(windows)]
        let prompt = "  Are you sure you want to uninstall nd300 and speedqx? (y/N): ";
        if !prompt_yes_no(prompt) {
            println!("  Uninstall cancelled.");
            return 0;
        }
        println!();
    }

    let report = execute_uninstall(&real_path).await;

    // Print results
    if report.target_retained {
        print_ok(
            "Invocation symlink removed; target installation retained",
            config,
        );
    } else if report.binary_removed {
        print_ok("nd300 binary removed", config);
    } else if report.binary_removal_scheduled {
        // Windows: the running exe is deleted by the spawned helper once we exit.
        print_ok(
            "nd300 binary scheduled for removal (completes when this process exits)",
            config,
        );
    } else {
        print_fail("Failed to remove nd300 binary", config);
    }

    if report.sibling_removed {
        print_ok("speedqx binary removed", config);
    }

    if report.receipt_removed {
        print_ok("Install receipt cleaned up", config);
    }

    if report.path_cleaned {
        print_ok("PATH entry removed", config);
    }

    for note in &report.notes {
        println!("  {}", color::dim(note, config));
    }

    println!();
    if report.target_retained {
        println!(
            "  {} {}",
            color::green(success_icon(config), config),
            color::green(
                "Invocation symlink removed; ND300 remains installed",
                config
            ),
        );
        0
    } else if report.binary_removed || report.binary_removal_scheduled {
        println!(
            "  {} {}",
            color::green(success_icon(config), config),
            color::green("nd300 and speedqx have been uninstalled", config),
        );
        0
    } else {
        println!(
            "  {} {}",
            color::red(fail_icon(config), config),
            color::red("Uninstall incomplete — could not remove binary", config),
        );
        2
    }
}

#[cfg(unix)]
fn unix_uninstall_cleans_receipt() -> bool {
    use super::unix_install::UnixOriginKind;
    let Ok(user) = crate::platform::invoking_user::InvokingUser::detect() else {
        return false;
    };
    super::unix_install::detect_origin(&user)
        .ok()
        .is_some_and(|origin| {
            matches!(
                origin.kind,
                UnixOriginKind::Cargo | UnixOriginKind::ManagedArchive
            ) && super::unix_install::cargo_dist_receipt_valid(&user)
        })
}

#[cfg(unix)]
fn print_unix_uninstall_preview(real_path: &Path, config: &Config) {
    use super::unix_install::UnixOriginKind;

    let origin = crate::platform::invoking_user::InvokingUser::detect()
        .ok()
        .and_then(|user| super::unix_install::detect_origin(&user).ok());
    match origin {
        Some(origin) if origin.kind == UnixOriginKind::Symlink => {
            let link = origin.invocation_symlink.as_deref().unwrap_or(real_path);
            println!(
                "  This will remove only the invocation symlink: {}",
                color::cyan(&link.display().to_string(), config)
            );
            println!("  The target installation will remain intact.");
        }
        Some(origin) if origin.kind == UnixOriginKind::Cargo => println!(
            "  This will run `cargo uninstall nd300` for: {}",
            color::cyan(&origin.executable.display().to_string(), config)
        ),
        Some(origin) if origin.kind == UnixOriginKind::ManagedArchive => println!(
            "  This will remove the validated ND300 archive install from: {}",
            color::cyan(
                &origin
                    .executable
                    .parent()
                    .unwrap_or(&origin.executable)
                    .display()
                    .to_string(),
                config
            )
        ),
        Some(origin) => println!(
            "  ND300 will inspect and refuse this {} installation unless its original manager removes it: {}",
            match origin.kind {
                UnixOriginKind::PackageManager => "package-manager-owned",
                UnixOriginKind::LocalBuild => "local-build",
                _ => "unknown",
            },
            color::cyan(&origin.executable.display().to_string(), config)
        ),
        None => println!(
            "  ND300 could not validate the install origin and will refuse removal: {}",
            color::cyan(&real_path.display().to_string(), config)
        ),
    }
}

async fn run_json(exe_path: &Path, _config: &Config) -> i32 {
    let report = execute_uninstall(exe_path).await;

    // `binary_removed` stays a literal "is it gone right now?" so existing
    // scripts read the same field; `success` and the exit code also accept a
    // Windows scheduled removal or a Unix symlink-only removal.
    let succeeded =
        report.binary_removed || report.binary_removal_scheduled || report.target_retained;
    let output = serde_json::json!({
        "action": "uninstall",
        "success": succeeded,
        "binary_removed": report.binary_removed,
        "binary_removal_scheduled": report.binary_removal_scheduled,
        "sibling_removed": report.sibling_removed,
        "receipt_removed": report.receipt_removed,
        "path_cleaned": report.path_cleaned,
        "notes": report.notes,
        "path": exe_path.display().to_string(),
    });
    println!(
        "{}",
        serde_json::to_string_pretty(&output).unwrap_or_else(|_| "{}".to_string())
    );

    if succeeded {
        0
    } else {
        2
    }
}

async fn execute_uninstall(_exe_path: &Path) -> CleanupReport {
    #[cfg(unix)]
    {
        let user = match crate::platform::invoking_user::InvokingUser::detect() {
            Ok(user) => user,
            Err(error) => {
                return CleanupReport {
                    binary_removed: false,
                    binary_removal_scheduled: false,
                    target_retained: false,
                    sibling_removed: false,
                    receipt_removed: false,
                    path_cleaned: false,
                    notes: vec![format!("Could not identify invoking user: {error}")],
                };
            }
        };
        super::unix_install::uninstall_detected(&user).await
    }

    #[cfg(windows)]
    {
        uninstall_path(_exe_path)
    }
}

#[cfg(windows)]
pub(crate) fn uninstall_path(exe_path: &Path) -> CleanupReport {
    let mut report = CleanupReport {
        binary_removed: false,
        binary_removal_scheduled: false,
        target_retained: false,
        sibling_removed: false,
        receipt_removed: false,
        path_cleaned: false,
        notes: Vec::new(),
    };

    // Step 1: Remove the receipt/config directory
    // This is safe to do first since it's nd300-specific
    if let Some(receipt_dir) = get_receipt_dir() {
        if receipt_dir.exists() {
            match std::fs::remove_dir_all(&receipt_dir) {
                Ok(_) => report.receipt_removed = true,
                Err(e) => report.notes.push(format!(
                    "Could not remove receipt dir {}: {}",
                    receipt_dir.display(),
                    e
                )),
            }
        }
    }

    // Step 2: Remove sibling binaries (speedqx) from the same directory
    if let Some(bin_dir) = exe_path.parent() {
        let exe_name = exe_path
            .file_name()
            .map(|n| n.to_string_lossy().to_lowercase())
            .unwrap_or_default();

        for name in OUR_BINARIES {
            let sibling = if cfg!(windows) {
                bin_dir.join(format!("{}.exe", name))
            } else {
                bin_dir.join(name)
            };

            let sibling_lower = sibling
                .file_name()
                .map(|n| n.to_string_lossy().to_lowercase())
                .unwrap_or_default();

            // Skip the current exe (handled separately in step 4)
            if sibling_lower == exe_name {
                continue;
            }

            if sibling.exists() {
                match std::fs::remove_file(&sibling) {
                    Ok(_) => report.sibling_removed = true,
                    Err(e) => {
                        report
                            .notes
                            .push(format!("Could not remove {}: {}", sibling.display(), e))
                    }
                }
            }
        }
    }

    // Step 3: Clean up PATH on Windows
    // Only remove the bin dir from PATH if only our binaries were there
    #[cfg(windows)]
    {
        let bin_dir = exe_path.parent().map(|p| p.to_path_buf());
        if let Some(ref dir) = bin_dir {
            if is_sole_package_in_dir(dir) {
                match remove_from_user_path(dir) {
                    Ok(true) => report.path_cleaned = true,
                    Ok(false) => {} // wasn't in PATH, nothing to do
                    Err(e) => report.notes.push(format!("Could not clean PATH: {}", e)),
                }
            } else {
                report.notes.push(
                    "Other binaries share the install directory — PATH entry left intact"
                        .to_string(),
                );
            }
        }
    }

    // Step 4: Remove the binary itself (must be last)
    #[cfg(unix)]
    {
        // On Unix, a running binary can be unlinked — the inode persists until exit
        match std::fs::remove_file(exe_path) {
            Ok(_) => report.binary_removed = true,
            Err(e) => report.notes.push(format!("Failed to remove binary: {}", e)),
        }
    }

    #[cfg(windows)]
    {
        // On Windows, a running exe cannot be deleted directly.
        // Spawn a background cmd that waits briefly, then deletes the file.
        // The file is NOT gone yet, so we set `binary_removal_scheduled` (not
        // `binary_removed`) — the updater's shadow guard depends on the
        // distinction.
        match build_delayed_delete_command(exe_path) {
            Some(script) => {
                match std::process::Command::new("cmd")
                    .args(["/C", &script])
                    .stdin(std::process::Stdio::null())
                    .stdout(std::process::Stdio::null())
                    .stderr(std::process::Stdio::null())
                    .spawn()
                {
                    Ok(_) => report.binary_removal_scheduled = true,
                    Err(e) => report
                        .notes
                        .push(format!("Failed to spawn cleanup process: {}", e)),
                }
            }
            None => {
                // The path contains characters that can't be safely embedded in a
                // `cmd /C` string. Refuse rather than risk a malformed/dangerous
                // command — neither flag is set, so this is reported as a failure.
                report.notes.push(format!(
                    "Could not schedule binary removal: path contains characters unsafe for a Windows shell command: {}",
                    exe_path.display()
                ));
            }
        }
    }

    report
}

/// Build the `cmd /C` script that deletes `exe_path` shortly after the current
/// process exits (Windows can't delete a running exe in place).
///
/// Returns `None` if the path contains any character that is either illegal in a
/// real Windows path or unsafe to embed in a `cmd` command line — `"`, newline,
/// carriage return, or any cmd metacharacter (`& ^ | < >`). All of these are
/// already illegal in genuine Windows file paths, so `None` is an honest refusal
/// rather than a real limitation. `%` is escaped to `%%` so `cmd`'s
/// environment-variable expansion can't mangle a path containing a literal `%`.
#[cfg(windows)]
fn build_delayed_delete_command(exe_path: &Path) -> Option<String> {
    let raw = exe_path.to_string_lossy();

    // Reject anything that would break out of the quoted `del "..."` argument or
    // that cmd would interpret as a metacharacter.
    const FORBIDDEN: [char; 8] = ['"', '\n', '\r', '&', '^', '|', '<', '>'];
    if raw.chars().any(|c| FORBIDDEN.contains(&c)) {
        return None;
    }

    // Escape `%` so cmd doesn't try to expand `%VAR%`-style sequences in the path.
    let safe = raw.replace('%', "%%");
    Some(format!("ping localhost -n 3 > nul & del \"{}\"", safe))
}

fn print_ok(label: &str, config: &Config) {
    println!(
        "  {} {}",
        color::green(success_icon(config), config),
        color::green(label, config),
    );
}

fn print_fail(label: &str, config: &Config) {
    println!(
        "  {} {}",
        color::red(fail_icon(config), config),
        color::red(label, config),
    );
}

/// Get the cargo-dist receipt directory for nd300.
/// - Windows: %LOCALAPPDATA%\nd300
/// - macOS/Linux: ~/.config/nd300  (XDG_CONFIG_HOME respected)
fn get_receipt_dir() -> Option<PathBuf> {
    #[cfg(windows)]
    {
        std::env::var("LOCALAPPDATA")
            .ok()
            .map(|base| PathBuf::from(base).join("nd300"))
    }

    #[cfg(not(windows))]
    {
        crate::platform::invoking_user::InvokingUser::detect()
            .ok()
            .map(|user| {
                let config_home = if !user.is_different_from_effective_user() {
                    std::env::var_os("XDG_CONFIG_HOME")
                        .map(PathBuf::from)
                        .filter(|path| path.is_absolute())
                        .unwrap_or_else(|| user.home().join(".config"))
                } else {
                    user.home().join(".config")
                };
                config_home.join("nd300")
            })
    }
}

/// Check if only our package's binaries (nd300, speedqx) are in the given directory.
/// If other binaries are present (e.g. cargo, rustup), we must NOT remove the
/// directory from PATH — that would break the user's Rust toolchain.
#[cfg(windows)]
pub(crate) fn is_sole_package_in_dir(dir: &Path) -> bool {
    let our_names: Vec<String> = OUR_BINARIES.iter().map(|n| format!("{}.exe", n)).collect();

    match std::fs::read_dir(dir) {
        Ok(entries) => {
            let other_exes: Vec<_> = entries
                .filter_map(|e| e.ok())
                .filter(|e| {
                    let name = e.file_name().to_string_lossy().to_lowercase();
                    name.ends_with(".exe") && !our_names.contains(&name)
                })
                .collect();
            other_exes.is_empty()
        }
        Err(_) => false,
    }
}

/// True if a single PATH entry refers to the same directory as `target`,
/// ignoring surrounding whitespace, ASCII case, and a trailing `\` or `/`.
///
/// Only the comparison is normalized — callers keep the original (untrimmed)
/// slice for any entry they retain, so non-matching paths are never rewritten.
#[cfg(windows)]
fn path_entry_matches_target(entry: &str, target: &str) -> bool {
    let norm = |s: &str| -> String { s.trim().trim_end_matches(['\\', '/']).to_lowercase() };
    let entry_norm = norm(entry);
    // An empty entry (e.g. a stray `;;`) never matches a real target dir.
    !entry_norm.is_empty() && entry_norm == norm(target)
}

/// Remove a directory from the user-level PATH environment variable (Windows registry).
/// Returns Ok(true) if the entry was found and removed, Ok(false) if it wasn't in PATH.
#[cfg(windows)]
fn remove_from_user_path(dir_to_remove: &Path) -> Result<bool, String> {
    use std::process::Command;

    // Read current user PATH from registry
    let output = Command::new("reg")
        .args(["query", "HKCU\\Environment", "/v", "PATH"])
        .output()
        .map_err(|e| format!("Failed to query registry: {}", e))?;

    let text = String::from_utf8_lossy(&output.stdout);
    // Parse the PATH value and its registry type from reg query output
    // Format: "    PATH    REG_EXPAND_SZ    value"
    let (current_path, reg_type) = match text.lines().find(|line| {
        line.contains("PATH") && (line.contains("REG_EXPAND_SZ") || line.contains("REG_SZ"))
    }) {
        Some(line) => {
            if let Some(idx) = line.find("REG_EXPAND_SZ") {
                (
                    line[idx + "REG_EXPAND_SZ".len()..].trim().to_string(),
                    "REG_EXPAND_SZ",
                )
            } else if let Some(idx) = line.find("REG_SZ") {
                (line[idx + "REG_SZ".len()..].trim().to_string(), "REG_SZ")
            } else {
                return Ok(false);
            }
        }
        None => return Ok(false), // No user PATH set
    };

    let dir_str = dir_to_remove.to_string_lossy();
    // Filter out the directory we want to remove. The comparison is
    // case-insensitive AND trailing-slash-insensitive (mirroring `same_path` in
    // update.rs) so a PATH entry with a trailing `\` or `/` still matches and is
    // removed — but we keep the ORIGINAL (untrimmed) slices for any entries we
    // retain, so we never rewrite paths we aren't removing.
    let new_parts: Vec<&str> = current_path
        .split(';')
        .filter(|part| !path_entry_matches_target(part, &dir_str))
        .filter(|part| !part.trim().is_empty())
        .collect();

    let original_count = current_path
        .split(';')
        .filter(|p| !p.trim().is_empty())
        .count();

    if new_parts.len() == original_count {
        return Ok(false); // Directory wasn't in PATH
    }

    let new_path = new_parts.join(";");

    // Write updated PATH back to registry
    let status = Command::new("reg")
        .args([
            "add",
            "HKCU\\Environment",
            "/v",
            "PATH",
            "/t",
            reg_type,
            "/d",
            &new_path,
            "/f",
        ])
        .output()
        .map_err(|e| format!("Failed to update registry: {}", e))?;

    if status.status.success() {
        // Broadcast WM_SETTINGCHANGE so Explorer picks up the change
        let _ = Command::new("powershell")
            .args([
                "-NoProfile",
                "-Command",
                "Add-Type -Namespace Win32 -Name NativeMethods -MemberDefinition '[DllImport(\"user32.dll\", SetLastError = true, CharSet = CharSet.Auto)] public static extern IntPtr SendMessageTimeout(IntPtr hWnd, uint Msg, UIntPtr wParam, string lParam, uint fuFlags, uint uTimeout, out UIntPtr lpdwResult);'; $HWND_BROADCAST = [IntPtr]0xffff; $WM_SETTINGCHANGE = 0x1a; $result = [UIntPtr]::Zero; [Win32.NativeMethods]::SendMessageTimeout($HWND_BROADCAST, $WM_SETTINGCHANGE, [UIntPtr]::Zero, 'Environment', 2, 5000, [ref]$result)",
            ])
            .output();
        Ok(true)
    } else {
        Err("Failed to write updated PATH to registry".to_string())
    }
}

#[cfg(test)]
mod tests {
    #[cfg(windows)]
    use super::*;

    // ── L3: PATH-entry matching ignores case, whitespace, and trailing slash ──
    #[cfg(windows)]
    #[test]
    fn path_entry_matches_target_variants() {
        let target = r"C:\x\bin";

        // Exact, trailing-backslash, trailing-forward-slash, mixed case, and
        // surrounding whitespace all match.
        assert!(path_entry_matches_target(r"C:\x\bin", target));
        assert!(path_entry_matches_target(r"C:\x\bin\", target));
        assert!(path_entry_matches_target("C:\\x\\bin/", target));
        assert!(path_entry_matches_target(r"c:\X\BIN", target));
        assert!(path_entry_matches_target("  C:\\x\\bin\\  ", target));
        assert!(path_entry_matches_target(r"C:\x\bin//", target));

        // A trailing slash on the TARGET side is normalized too.
        assert!(path_entry_matches_target(r"C:\x\bin", r"C:\x\bin\"));

        // A different (longer) directory must NOT match.
        assert!(!path_entry_matches_target(r"C:\x\bingo", target));
        assert!(!path_entry_matches_target(r"C:\x", target));
        assert!(!path_entry_matches_target("", target));
        assert!(!path_entry_matches_target("   ", target));
    }

    // ── L4: delayed-delete command is built safely or honestly refused ────────
    #[cfg(windows)]
    #[test]
    fn build_delayed_delete_command_normal_path() {
        let cmd = build_delayed_delete_command(Path::new(r"C:\Users\me\.cargo\bin\nd300.exe"))
            .expect("a normal path yields a command");
        assert!(cmd.contains(r#"del "C:\Users\me\.cargo\bin\nd300.exe""#));
        assert!(cmd.starts_with("ping localhost -n 3 > nul & del \""));
    }

    #[cfg(windows)]
    #[test]
    fn build_delayed_delete_command_escapes_percent() {
        let cmd = build_delayed_delete_command(Path::new(r"C:\weird%path\nd300.exe"))
            .expect("a percent in a path is escaped, not refused");
        assert!(cmd.contains(r"C:\weird%%path\nd300.exe"));
        // The single literal percent must not survive un-escaped.
        assert!(!cmd.contains(r"weird%path"));
    }

    #[cfg(windows)]
    #[test]
    fn build_delayed_delete_command_refuses_metacharacters() {
        // Every cmd metacharacter / quote / newline yields a refusal (None).
        for bad in [
            r#"C:\a"b\nd300.exe"#,
            r"C:\a&b\nd300.exe",
            r"C:\a^b\nd300.exe",
            r"C:\a|b\nd300.exe",
            r"C:\a<b\nd300.exe",
            r"C:\a>b\nd300.exe",
            "C:\\a\nb\\nd300.exe",
            "C:\\a\rb\\nd300.exe",
        ] {
            assert!(
                build_delayed_delete_command(Path::new(bad)).is_none(),
                "expected refusal for path: {:?}",
                bad
            );
        }
    }
}
