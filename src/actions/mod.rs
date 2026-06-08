pub mod clear_dns;
pub mod dns;
pub mod fix;
pub mod migrate;
pub mod uninstall;
pub mod update;

use crate::config::{Config, OutputFormat};
use std::io::IsTerminal;

use fix::cmd::{run_cmd, TIMEOUT_QUICK};

/// Run the platform-specific DNS flush command.
/// Returns Ok(stdout message) on success, Err(stderr/error message) on failure.
pub async fn flush_dns_platform() -> Result<String, String> {
    #[cfg(windows)]
    {
        let mut cmd = tokio::process::Command::new("ipconfig");
        cmd.arg("/flushdns");
        match run_cmd(cmd, TIMEOUT_QUICK).await {
            Ok(output) if output.status.success() => {
                Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
            }
            Ok(output) => Err(String::from_utf8_lossy(&output.stderr).trim().to_string()),
            Err(e) => Err(e),
        }
    }

    #[cfg(target_os = "macos")]
    {
        // dscacheutil -flushcache
        let mut flush_cmd = tokio::process::Command::new("dscacheutil");
        flush_cmd.arg("-flushcache");
        let flush = run_cmd(flush_cmd, TIMEOUT_QUICK).await;
        // killall -HUP mDNSResponder
        let mut kill_cmd = tokio::process::Command::new("killall");
        kill_cmd.args(["-HUP", "mDNSResponder"]);
        let kill = run_cmd(kill_cmd, TIMEOUT_QUICK).await;

        let flush_ok = matches!(&flush, Ok(f) if f.status.success());
        let kill_ok = matches!(&kill, Ok(k) if k.status.success());

        match (flush_ok, kill_ok) {
            (true, true) => Ok("DNS cache flushed successfully".to_string()),
            (true, false) => Ok("DNS cache flushed (user cache only)".to_string()),
            (false, true) => Ok("DNS cache flushed (mDNSResponder restarted)".to_string()),
            (false, false) => {
                // Return the most useful error message
                match (flush, kill) {
                    (Err(e), _) | (_, Err(e)) => Err(e),
                    (Ok(f), _) if !f.status.success() => {
                        Err(String::from_utf8_lossy(&f.stderr).trim().to_string())
                    }
                    (_, Ok(k)) if !k.status.success() => {
                        Err(String::from_utf8_lossy(&k.stderr).trim().to_string())
                    }
                    _ => Err("Failed to flush DNS".to_string()),
                }
            }
        }
    }

    #[cfg(target_os = "linux")]
    {
        let mut flushed = Vec::new();

        // Layer 1: systemd-resolved (resolvectl, fallback systemd-resolve)
        let mut check_cmd = tokio::process::Command::new("systemctl");
        check_cmd.args(["is-active", "systemd-resolved"]);
        if let Ok(output) = run_cmd(check_cmd, TIMEOUT_QUICK).await {
            if String::from_utf8_lossy(&output.stdout).trim() == "active" {
                let mut resolvectl_cmd = tokio::process::Command::new("resolvectl");
                resolvectl_cmd.arg("flush-caches");
                let ok = if let Ok(r) = run_cmd(resolvectl_cmd, TIMEOUT_QUICK).await {
                    r.status.success()
                } else {
                    false
                };

                if !ok {
                    let mut fallback_cmd = tokio::process::Command::new("systemd-resolve");
                    fallback_cmd.arg("--flush-caches");
                    if let Ok(r) = run_cmd(fallback_cmd, TIMEOUT_QUICK).await {
                        if r.status.success() {
                            flushed.push("systemd-resolved");
                        }
                    }
                } else {
                    flushed.push("systemd-resolved");
                }
            }
        }

        // Layer 2: dnsmasq (often a NetworkManager plugin)
        let mut pgrep_dnsmasq = tokio::process::Command::new("pgrep");
        pgrep_dnsmasq.arg("dnsmasq");
        if let Ok(output) = run_cmd(pgrep_dnsmasq, TIMEOUT_QUICK).await {
            if output.status.success() {
                let mut killall_cmd = tokio::process::Command::new("killall");
                killall_cmd.args(["-HUP", "dnsmasq"]);
                if let Ok(r) = run_cmd(killall_cmd, TIMEOUT_QUICK).await {
                    if r.status.success() {
                        flushed.push("dnsmasq");
                    }
                }
            }
        }

        // Layer 3: nscd
        let mut pgrep_nscd = tokio::process::Command::new("pgrep");
        pgrep_nscd.arg("nscd");
        if let Ok(output) = run_cmd(pgrep_nscd, TIMEOUT_QUICK).await {
            if output.status.success() {
                let mut nscd_cmd = tokio::process::Command::new("nscd");
                nscd_cmd.args(["-i", "hosts"]);
                if let Ok(r) = run_cmd(nscd_cmd, TIMEOUT_QUICK).await {
                    if r.status.success() {
                        flushed.push("nscd");
                    }
                }
            }
        }

        if flushed.is_empty() {
            Err("No DNS caching service detected or flush failed".to_string())
        } else {
            Ok(format!("Flushed: {}", flushed.join(", ")))
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

/// Prompt the user for free-text input (e.g. SSID, passphrase).
pub fn prompt_string(prompt: &str) -> String {
    use std::io::Write;
    print!("{}", prompt);
    let _ = std::io::stdout().flush();
    let mut input = String::new();
    let _ = std::io::stdin().read_line(&mut input);
    input.trim().to_string()
}
