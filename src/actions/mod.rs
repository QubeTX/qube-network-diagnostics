pub mod clear_dns;
pub mod fix;
pub mod uninstall;

use crate::config::{Config, OutputFormat};
use std::io::IsTerminal;

/// Run the platform-specific DNS flush command.
/// Returns Ok(stdout message) on success, Err(stderr/error message) on failure.
pub async fn flush_dns_platform() -> Result<String, String> {
    #[cfg(windows)]
    {
        match tokio::process::Command::new("ipconfig")
            .arg("/flushdns")
            .output()
            .await
        {
            Ok(output) if output.status.success() => {
                Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
            }
            Ok(output) => Err(String::from_utf8_lossy(&output.stderr).trim().to_string()),
            Err(e) => Err(format!("Failed to run ipconfig: {}", e)),
        }
    }

    #[cfg(target_os = "macos")]
    {
        // dscacheutil -flushcache
        let flush = tokio::process::Command::new("dscacheutil")
            .arg("-flushcache")
            .output()
            .await;
        // killall -HUP mDNSResponder
        let kill = tokio::process::Command::new("killall")
            .args(["-HUP", "mDNSResponder"])
            .output()
            .await;

        match (flush, kill) {
            (Ok(f), Ok(k)) if f.status.success() && k.status.success() => {
                Ok("DNS cache flushed successfully".to_string())
            }
            (Ok(f), _) if !f.status.success() => {
                Err(String::from_utf8_lossy(&f.stderr).trim().to_string())
            }
            (_, Ok(k)) if !k.status.success() => {
                Err(String::from_utf8_lossy(&k.stderr).trim().to_string())
            }
            (Err(e), _) | (_, Err(e)) => Err(format!("Failed to flush DNS: {}", e)),
        }
    }

    #[cfg(target_os = "linux")]
    {
        // Try resolvectl first, fall back to systemd-resolve
        let result = tokio::process::Command::new("resolvectl")
            .arg("flush-caches")
            .output()
            .await;

        match result {
            Ok(output) if output.status.success() => {
                Ok("DNS cache flushed successfully".to_string())
            }
            _ => {
                // Fallback
                match tokio::process::Command::new("systemd-resolve")
                    .arg("--flush-caches")
                    .output()
                    .await
                {
                    Ok(output) if output.status.success() => {
                        Ok("DNS cache flushed successfully".to_string())
                    }
                    Ok(output) => {
                        Err(String::from_utf8_lossy(&output.stderr).trim().to_string())
                    }
                    Err(e) => Err(format!("No DNS flush command available: {}", e)),
                }
            }
        }
    }
}

/// Prompt the user with a yes/no question. Returns true if they answer 'y' or 'Y'.
/// Default is No (returns false on empty input).
pub fn prompt_yes_no(prompt: &str) -> bool {
    use std::io::Write;
    print!("{}", prompt);
    let _ = std::io::stdout().flush();
    let mut input = String::new();
    if std::io::stdin().read_line(&mut input).is_ok() {
        matches!(input.trim(), "y" | "Y" | "yes" | "Yes" | "YES")
    } else {
        false
    }
}

/// Returns true if we should show interactive prompts (not JSON, stdin is a TTY).
pub fn is_interactive(config: &Config) -> bool {
    config.format != OutputFormat::Json && std::io::stdin().is_terminal()
}

/// Return the success icon character respecting --ascii mode.
pub fn success_icon(config: &Config) -> &'static str {
    if config.use_unicode {
        crate::config::status_chars::OK
    } else {
        crate::config::status_chars::OK_ASCII
    }
}

/// Return the fail icon character respecting --ascii mode.
pub fn fail_icon(config: &Config) -> &'static str {
    if config.use_unicode {
        crate::config::status_chars::FAIL
    } else {
        crate::config::status_chars::FAIL_ASCII
    }
}

/// Print a platform-specific hint about running with elevated privileges.
pub fn print_elevation_hint(config: &Config) {
    let hint = if cfg!(windows) {
        "Run as Administrator for full network reset capabilities"
    } else {
        "Run with sudo for full network reset capabilities"
    };
    println!("  {}", crate::render::color::dim(hint, config));
}
