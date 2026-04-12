use crate::config::{Config, OutputFormat};
use crate::render::color;
use crate::VERSION;

use super::{fail_icon, success_icon};

/// GitHub API endpoint for the latest release.
const RELEASES_URL: &str =
    "https://api.github.com/repos/QubeTX/qube-network-diagnostics/releases/latest";

/// Shell installer URL (macOS/Linux).
#[cfg(not(windows))]
const SHELL_INSTALLER: &str =
    "https://github.com/QubeTX/qube-network-diagnostics/releases/latest/download/nd-300-installer.sh";

/// PowerShell installer URL (Windows).
const PS_INSTALLER: &str =
    "https://github.com/QubeTX/qube-network-diagnostics/releases/latest/download/nd-300-installer.ps1";

/// Run the self-update flow.
pub async fn run(config: &Config) -> i32 {
    if config.format == OutputFormat::Json {
        return run_json(config).await;
    }

    println!();
    println!(
        "  {} Checking for updates...",
        color::cyan("*", config),
    );

    // ── Fetch latest version from GitHub ────────────────────────────
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
            color::green(&format!("Already on the latest version (v{})", current), config),
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

    // ── Detect installation method ──────────────────────────────────
    let method = detect_install_method();
    let method_label = match method {
        InstallMethod::Cargo => "cargo install",
        InstallMethod::Installer => {
            if cfg!(windows) { "PowerShell installer" } else { "shell installer" }
        }
    };

    println!(
        "  {} Updating via {}...",
        color::cyan("*", config),
        method_label,
    );
    println!();

    // ── Execute update ──────────────────────────────────────────────
    match execute_update(&method).await {
        Ok(()) => {
            println!();
            println!(
                "  {} {}",
                color::green(success_icon(config), config),
                color::green(&format!("Updated to v{}", latest), config),
            );
            0
        }
        Err(e) => {
            println!();
            println!(
                "  {} {}",
                color::red(fail_icon(config), config),
                color::red(&format!("Update failed: {}", e), config),
            );
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
            println!("{}", serde_json::to_string_pretty(&output).unwrap_or_else(|_| "{}".to_string()));
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
        println!("{}", serde_json::to_string_pretty(&output).unwrap_or_else(|_| "{}".to_string()));
        return 0;
    }

    let method = detect_install_method();
    match execute_update(&method).await {
        Ok(()) => {
            let output = serde_json::json!({
                "action": "update",
                "success": true,
                "message": format!("Updated from v{} to v{}", current, latest),
                "current_version": current,
                "latest_version": latest,
                "update_available": true,
                "method": match method {
                    InstallMethod::Cargo => "cargo",
                    InstallMethod::Installer => "installer",
                },
            });
            println!("{}", serde_json::to_string_pretty(&output).unwrap_or_else(|_| "{}".to_string()));
            0
        }
        Err(e) => {
            let output = serde_json::json!({
                "action": "update",
                "success": false,
                "message": format!("Update failed: {}", e),
                "current_version": current,
                "latest_version": latest,
                "update_available": true,
            });
            println!("{}", serde_json::to_string_pretty(&output).unwrap_or_else(|_| "{}".to_string()));
            2
        }
    }
}

/// Fetch the latest version tag from GitHub releases API.
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

    // Strip leading 'v' if present
    Ok(tag.strip_prefix('v').unwrap_or(tag).to_string())
}

/// Compare semver versions. Returns true if `latest` is newer than `current`.
fn is_newer(current: &str, latest: &str) -> bool {
    let parse = |v: &str| -> Vec<u64> {
        v.split('.')
            .filter_map(|s| s.parse::<u64>().ok())
            .collect()
    };

    let c = parse(current);
    let l = parse(latest);

    // Pad shorter vec with zeros
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

enum InstallMethod {
    Cargo,
    Installer,
}

/// Detect how nd300 was installed by examining the binary's path.
fn detect_install_method() -> InstallMethod {
    if let Ok(exe) = std::env::current_exe() {
        let path_str = exe.to_string_lossy().to_lowercase();
        if path_str.contains(".cargo") && path_str.contains("bin") {
            return InstallMethod::Cargo;
        }
    }
    InstallMethod::Installer
}

/// Execute the platform-specific update command.
async fn execute_update(method: &InstallMethod) -> Result<(), String> {
    let status = match method {
        InstallMethod::Cargo => {
            std::process::Command::new("cargo")
                .args(["install", "nd-300", "--force"])
                .status()
                .map_err(|e| format!("Failed to run cargo install: {}", e))?
        }
        InstallMethod::Installer => {
            #[cfg(windows)]
            {
                std::process::Command::new("powershell")
                    .args([
                        "-ExecutionPolicy",
                        "ByPass",
                        "-c",
                        &format!("irm {} | iex", PS_INSTALLER),
                    ])
                    .status()
                    .map_err(|e| format!("Failed to run PowerShell installer: {}", e))?
            }

            #[cfg(not(windows))]
            {
                std::process::Command::new("sh")
                    .args([
                        "-c",
                        &format!(
                            "curl --proto '=https' --tlsv1.2 -LsSf {} | sh",
                            SHELL_INSTALLER
                        ),
                    ])
                    .status()
                    .map_err(|e| format!("Failed to run shell installer: {}", e))?
            }
        }
    };

    if status.success() {
        Ok(())
    } else {
        Err(format!("Installer exited with code {}", status.code().unwrap_or(-1)))
    }
}
