use crate::config::{Config, OutputFormat};
use crate::render::color;
use std::path::{Path, PathBuf};

use super::{fail_icon, is_interactive, prompt_yes_no, success_icon};

/// Tracks what we cleaned up for reporting.
struct CleanupReport {
    binary_removed: bool,
    receipt_removed: bool,
    path_cleaned: bool,
    notes: Vec<String>,
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
                println!("{}", serde_json::to_string_pretty(&output).unwrap_or_else(|_| "{}".to_string()));
            } else {
                println!(
                    "  {} {}",
                    color::red(fail_icon(config), config),
                    color::red(&format!("Could not determine binary location: {}", e), config),
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
    println!(
        "  This will remove nd300 from: {}",
        color::cyan(&real_path.display().to_string(), config),
    );

    // Show what we'll clean up
    let receipt_dir = get_receipt_dir();
    if let Some(ref dir) = receipt_dir {
        if dir.exists() {
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
            if is_sole_binary_in_dir(&real_path, dir) {
                println!(
                    "  PATH entry to clean:        {}",
                    color::cyan(&dir.display().to_string(), config),
                );
            }
        }
    }

    println!();

    if is_interactive(config) {
        if !prompt_yes_no("  Are you sure you want to uninstall nd300? (y/N): ") {
            println!("  Uninstall cancelled.");
            return 0;
        }
        println!();
    }

    let report = do_uninstall(&real_path, config).await;

    // Print results
    if report.binary_removed {
        print_ok("Binary removed", config);
    } else {
        print_fail("Failed to remove binary", config);
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
    if report.binary_removed {
        println!(
            "  {} {}",
            color::green(success_icon(config), config),
            color::green("nd300 has been uninstalled", config),
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

async fn run_json(exe_path: &Path, config: &Config) -> i32 {
    let report = do_uninstall(exe_path, config).await;

    let output = serde_json::json!({
        "action": "uninstall",
        "success": report.binary_removed,
        "binary_removed": report.binary_removed,
        "receipt_removed": report.receipt_removed,
        "path_cleaned": report.path_cleaned,
        "notes": report.notes,
        "path": exe_path.display().to_string(),
    });
    println!("{}", serde_json::to_string_pretty(&output).unwrap_or_else(|_| "{}".to_string()));

    if report.binary_removed { 0 } else { 2 }
}

async fn do_uninstall(exe_path: &Path, _config: &Config) -> CleanupReport {
    let mut report = CleanupReport {
        binary_removed: false,
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

    // Step 2: Clean up PATH on Windows
    // Only remove the bin dir from PATH if nd300 is the only binary there
    #[cfg(windows)]
    {
        let bin_dir = exe_path.parent().map(|p| p.to_path_buf());
        if let Some(ref dir) = bin_dir {
            if is_sole_binary_in_dir(exe_path, dir) {
                match remove_from_user_path(dir) {
                    Ok(true) => report.path_cleaned = true,
                    Ok(false) => {} // wasn't in PATH, nothing to do
                    Err(e) => report.notes.push(format!("Could not clean PATH: {}", e)),
                }
            } else {
                report.notes.push(
                    "Other binaries share the install directory — PATH entry left intact".to_string(),
                );
            }
        }
    }

    // Step 3: Remove the binary itself (must be last)
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
        let exe_str = exe_path.to_string_lossy();
        let script = format!(
            "ping localhost -n 3 > nul & del \"{}\"",
            exe_str
        );

        match std::process::Command::new("cmd")
            .args(["/C", &script])
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
        {
            Ok(_) => report.binary_removed = true,
            Err(e) => report.notes.push(format!("Failed to spawn cleanup process: {}", e)),
        }
    }

    report
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
        let base = std::env::var("XDG_CONFIG_HOME")
            .ok()
            .map(PathBuf::from)
            .or_else(|| {
                std::env::var("HOME")
                    .ok()
                    .map(|h| PathBuf::from(h).join(".config"))
            });
        base.map(|b| b.join("nd300"))
    }
}

/// Check if nd300 is the only executable in the given directory.
/// If other binaries are present (e.g. cargo, rustup), we must NOT remove the
/// directory from PATH — that would break the user's Rust toolchain.
#[cfg(windows)]
fn is_sole_binary_in_dir(exe_path: &Path, dir: &Path) -> bool {
    let exe_name = exe_path
        .file_name()
        .map(|n| n.to_string_lossy().to_lowercase());

    match std::fs::read_dir(dir) {
        Ok(entries) => {
            let other_exes: Vec<_> = entries
                .filter_map(|e| e.ok())
                .filter(|e| {
                    let name = e.file_name().to_string_lossy().to_lowercase();
                    // Only count .exe files that aren't nd300
                    name.ends_with(".exe")
                        && Some(name.as_str().to_string()) != exe_name.as_ref().map(|n| n.to_string())
                })
                .collect();
            other_exes.is_empty()
        }
        Err(_) => false, // If we can't read the dir, don't touch PATH
    }
}

/// Remove a directory from the user-level PATH environment variable (Windows registry).
/// Returns Ok(true) if the entry was found and removed, Ok(false) if it wasn't in PATH.
#[cfg(windows)]
fn remove_from_user_path(dir_to_remove: &Path) -> Result<bool, String> {
    use std::process::Command;

    // Read current user PATH from registry
    let output = Command::new("reg")
        .args([
            "query",
            "HKCU\\Environment",
            "/v",
            "PATH",
        ])
        .output()
        .map_err(|e| format!("Failed to query registry: {}", e))?;

    let text = String::from_utf8_lossy(&output.stdout);
    // Parse the PATH value and its registry type from reg query output
    // Format: "    PATH    REG_EXPAND_SZ    value"
    let (current_path, reg_type) = match text
        .lines()
        .find(|line| line.contains("PATH") && (line.contains("REG_EXPAND_SZ") || line.contains("REG_SZ")))
    {
        Some(line) => {
            if let Some(idx) = line.find("REG_EXPAND_SZ") {
                (line[idx + "REG_EXPAND_SZ".len()..].trim().to_string(), "REG_EXPAND_SZ")
            } else if let Some(idx) = line.find("REG_SZ") {
                (line[idx + "REG_SZ".len()..].trim().to_string(), "REG_SZ")
            } else {
                return Ok(false);
            }
        }
        None => return Ok(false), // No user PATH set
    };

    let dir_str = dir_to_remove.to_string_lossy();
    // Filter out the directory we want to remove (case-insensitive on Windows)
    let dir_lower = dir_str.to_lowercase();
    let new_parts: Vec<&str> = current_path
        .split(';')
        .filter(|part| {
            let part_clean = part.trim();
            !part_clean.is_empty() && part_clean.to_lowercase() != dir_lower
        })
        .collect();

    let original_count = current_path.split(';').filter(|p| !p.trim().is_empty()).count();

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
