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
        // Try airport -I
        let airport_path = "/System/Library/PrivateFrameworks/Apple80211.framework/Versions/Current/Resources/airport";
        let mut cmd = tokio::process::Command::new(airport_path);
        cmd.arg("-I");
        if let Ok(output) = run_cmd(cmd, TIMEOUT_QUICK).await {
            let text = String::from_utf8_lossy(&output.stdout);
            for line in text.lines() {
                let trimmed = line.trim();
                if let Some(rest) = trimmed.strip_prefix("SSID:") {
                    let ssid = rest.trim().to_string();
                    if !ssid.is_empty() {
                        return Some(ssid);
                    }
                }
            }
        }
        // Fallback: networksetup
        let mut cmd2 = tokio::process::Command::new("networksetup");
        cmd2.args(["-getairportnetwork", "en0"]);
        if let Ok(output) = run_cmd(cmd2, TIMEOUT_QUICK).await {
            let text = String::from_utf8_lossy(&output.stdout);
            // "Current Wi-Fi Network: MyNetwork"
            if let Some(rest) = text.strip_prefix("Current Wi-Fi Network: ") {
                let ssid = rest.trim().to_string();
                if !ssid.is_empty() {
                    return Some(ssid);
                }
            }
        }
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
        let airport_path = "/System/Library/PrivateFrameworks/Apple80211.framework/Versions/Current/Resources/airport";
        let mut cmd = tokio::process::Command::new(airport_path);
        cmd.args(["-s"]);
        if let Ok(output) = run_cmd(cmd, TIMEOUT_QUICK).await {
            let text = String::from_utf8_lossy(&output.stdout);
            for line in text.lines().skip(1) {
                // First column is SSID (right-padded to ~33 chars)
                let ssid = line.get(..33).unwrap_or("").trim().to_string();
                if !ssid.is_empty() && !networks.contains(&ssid) {
                    networks.push(ssid);
                }
            }
        }
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
