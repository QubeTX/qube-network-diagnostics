#[allow(unused_imports)]
use super::cmd::{run_cmd, TIMEOUT_QUICK};

/// Capture the currently-connected Wi-Fi SSID before any disconnect operations.
pub async fn capture_current_ssid() -> Option<String> {
    #[cfg(windows)]
    {
        let mut cmd = tokio::process::Command::new("netsh");
        cmd.args(["wlan", "show", "interfaces"]);
        if let Ok(output) = run_cmd(cmd, TIMEOUT_QUICK).await {
            let text = String::from_utf8_lossy(&output.stdout);
            for line in text.lines() {
                let trimmed = line.trim();
                // Match "SSID" but not "BSSID"
                if let Some(rest) = trimmed.strip_prefix("SSID") {
                    if !rest.starts_with(' ') {
                        // This is "SSID  : value" not "BSSID : value"
                        // but the strip_prefix already consumed "SSID"
                    }
                    // Handle "SSID                   : MyNetwork"
                    if let Some(value) = rest.split(':').nth(1) {
                        let ssid = value.trim().to_string();
                        if !ssid.is_empty() {
                            return Some(ssid);
                        }
                    }
                }
            }
        }
        None
    }

    #[cfg(target_os = "macos")]
    {
        // The destructive macOS profile/service recreation path was removed.
        // It no longer needs an SSID, and must not depend on Apple's removed
        // private `airport` binary or attempt credential replay.
        None
    }

    #[cfg(target_os = "linux")]
    {
        let mut cmd = tokio::process::Command::new("iwgetid");
        cmd.arg("-r");
        if let Ok(output) = run_cmd(cmd, TIMEOUT_QUICK).await {
            if output.status.success() {
                let ssid = String::from_utf8_lossy(&output.stdout).trim().to_string();
                if !ssid.is_empty() {
                    return Some(ssid);
                }
            }
        }
        None
    }
}

/// Scan for available Wi-Fi networks (for reconnection after Stage 3).
pub async fn scan_wifi_networks() -> Vec<String> {
    #[allow(unused_mut)]
    let mut networks = Vec::new();

    #[cfg(windows)]
    {
        let mut cmd = tokio::process::Command::new("netsh");
        cmd.args(["wlan", "show", "networks"]);
        if let Ok(output) = run_cmd(cmd, TIMEOUT_QUICK).await {
            let text = String::from_utf8_lossy(&output.stdout);
            for line in text.lines() {
                let trimmed = line.trim();
                if let Some(rest) = trimmed.strip_prefix("SSID") {
                    if let Some(value) = rest.split(':').nth(1) {
                        let ssid = value.trim().to_string();
                        if !ssid.is_empty() && !networks.contains(&ssid) {
                            networks.push(ssid);
                        }
                    }
                }
            }
        }
    }

    #[cfg(target_os = "macos")]
    {
        // No macOS fix primitive scans/recreates Wi-Fi profiles anymore.
    }

    #[cfg(target_os = "linux")]
    {
        // Try nmcli
        let mut rescan_cmd = tokio::process::Command::new("nmcli");
        rescan_cmd.args(["device", "wifi", "rescan"]);
        let _ = run_cmd(rescan_cmd, TIMEOUT_QUICK).await;
        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
        let mut list_cmd = tokio::process::Command::new("nmcli");
        list_cmd.args(["-t", "-f", "SSID", "device", "wifi", "list"]);
        if let Ok(output) = run_cmd(list_cmd, TIMEOUT_QUICK).await {
            let text = String::from_utf8_lossy(&output.stdout);
            for line in text.lines() {
                let ssid = line.trim().to_string();
                if !ssid.is_empty() && !networks.contains(&ssid) {
                    networks.push(ssid);
                }
            }
        }
    }

    networks
}
