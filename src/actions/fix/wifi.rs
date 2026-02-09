/// Capture the currently-connected Wi-Fi SSID before any disconnect operations.
pub async fn capture_current_ssid() -> Option<String> {
    #[cfg(windows)]
    {
        if let Ok(output) = tokio::process::Command::new("netsh")
            .args(["wlan", "show", "interfaces"])
            .output()
            .await
        {
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
        if let Ok(output) = tokio::process::Command::new(airport_path)
            .arg("-I")
            .output()
            .await
        {
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
        if let Ok(output) = tokio::process::Command::new("networksetup")
            .args(["-getairportnetwork", "en0"])
            .output()
            .await
        {
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
        if let Ok(output) = tokio::process::Command::new("iwgetid")
            .arg("-r")
            .output()
            .await
        {
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
        if let Ok(output) = tokio::process::Command::new("netsh")
            .args(["wlan", "show", "networks"])
            .output()
            .await
        {
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
        if let Ok(output) = tokio::process::Command::new(airport_path)
            .args(["-s"])
            .output()
            .await
        {
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
        let _ = tokio::process::Command::new("nmcli")
            .args(["device", "wifi", "rescan"])
            .output()
            .await;
        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
        if let Ok(output) = tokio::process::Command::new("nmcli")
            .args(["-t", "-f", "SSID", "device", "wifi", "list"])
            .output()
            .await
        {
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
