/// List active network adapters — all platforms.
#[cfg(windows)]
pub async fn list_active_adapters() -> Vec<String> {
    use std::collections::HashMap;
    use wmi::{COMLibrary, WMIConnection};

    tokio::task::spawn_blocking(|| {
        let com = match COMLibrary::new() {
            Ok(c) => c,
            Err(_) => return Vec::new(),
        };
        let wmi = match WMIConnection::new(com) {
            Ok(w) => w,
            Err(_) => return Vec::new(),
        };

        let query = "SELECT NetConnectionID, NetConnectionStatus FROM Win32_NetworkAdapter WHERE NetConnectionID IS NOT NULL AND NetConnectionStatus = 2";
        let results: Vec<HashMap<String, wmi::Variant>> = match wmi.raw_query(query) {
            Ok(r) => r,
            Err(_) => return Vec::new(),
        };

        results
            .into_iter()
            .filter_map(|row| match row.get("NetConnectionID") {
                Some(wmi::Variant::String(s)) => Some(s.clone()),
                _ => None,
            })
            .collect()
    })
    .await
    .unwrap_or_default()
}

#[cfg(target_os = "macos")]
pub async fn list_active_adapters() -> Vec<String> {
    let mut adapters = Vec::new();
    if let Ok(output) = tokio::process::Command::new("networksetup")
        .args(["-listallhardwareports"])
        .output()
        .await
    {
        let text = String::from_utf8_lossy(&output.stdout);
        let mut current_name = String::new();

        for line in text.lines() {
            if let Some(name) = line.strip_prefix("Hardware Port: ") {
                current_name = name.trim().to_string();
            } else if let Some(dev) = line.strip_prefix("Device: ") {
                let device = dev.trim();
                // Check if this interface is active
                if let Ok(status_output) = tokio::process::Command::new("ifconfig")
                    .arg(device)
                    .output()
                    .await
                {
                    let status_text = String::from_utf8_lossy(&status_output.stdout);
                    if status_text.contains("status: active") || status_text.contains("inet ") {
                        adapters.push(format!("{}:{}", current_name, device));
                    }
                }
            }
        }
    }
    adapters
}

#[cfg(target_os = "linux")]
pub async fn list_active_adapters() -> Vec<String> {
    // Detect active non-loopback interfaces via `ip link show`
    let mut adapters = Vec::new();
    if let Ok(output) = tokio::process::Command::new("ip")
        .args(["link", "show", "up"])
        .output()
        .await
    {
        let text = String::from_utf8_lossy(&output.stdout);
        for line in text.lines() {
            // Lines like: "2: eth0: <BROADCAST,MULTICAST,UP,LOWER_UP> ..."
            if let Some(colon_idx) = line.find(": ") {
                let after = &line[colon_idx + 2..];
                if let Some(name_end) = after.find(':') {
                    let name = &after[..name_end];
                    if name != "lo" && !name.starts_with("veth") && !name.starts_with("docker") && !name.starts_with("br-") {
                        adapters.push(name.to_string());
                    }
                }
            }
        }
    }
    adapters
}

/// Restart a network adapter — all platforms.
#[cfg(windows)]
pub async fn restart_adapter(name: &str) -> Result<String, String> {
    // Disable
    let disable = tokio::process::Command::new("netsh")
        .args(["interface", "set", "interface", name, "disabled"])
        .output()
        .await;

    match disable {
        Ok(output) if !output.status.success() => {
            return Err(format!(
                "Failed to disable {}: {}",
                name,
                String::from_utf8_lossy(&output.stderr).trim()
            ));
        }
        Err(e) => return Err(format!("Failed to run netsh: {}", e)),
        _ => {}
    }

    // Wait for the adapter to fully disable
    tokio::time::sleep(std::time::Duration::from_secs(3)).await;

    // Re-enable
    match tokio::process::Command::new("netsh")
        .args(["interface", "set", "interface", name, "enabled"])
        .output()
        .await
    {
        Ok(output) if output.status.success() => Ok(format!("{} restarted", name)),
        Ok(output) => Err(format!(
            "Failed to re-enable {}: {}",
            name,
            String::from_utf8_lossy(&output.stderr).trim()
        )),
        Err(e) => Err(format!("Failed to run netsh: {}", e)),
    }
}

#[cfg(target_os = "macos")]
pub async fn restart_adapter(name_and_device: &str) -> Result<String, String> {
    // name_and_device is "HardwarePort:device" e.g. "Wi-Fi:en0"
    let parts: Vec<&str> = name_and_device.splitn(2, ':').collect();
    if parts.len() != 2 {
        return Err(format!("Invalid adapter format: {}", name_and_device));
    }
    let name = parts[0];
    let device = parts[1];

    let is_wifi = name.to_lowercase().contains("wi-fi") || name.to_lowercase().contains("airport");

    if is_wifi {
        // Soft toggle via networksetup
        let off = tokio::process::Command::new("networksetup")
            .args(["-setairportpower", device, "off"])
            .output()
            .await;

        if let Ok(output) = &off {
            if !output.status.success() {
                return Err(format!(
                    "Failed to disable Wi-Fi: {}",
                    String::from_utf8_lossy(&output.stderr).trim()
                ));
            }
        }

        tokio::time::sleep(std::time::Duration::from_secs(3)).await;

        match tokio::process::Command::new("networksetup")
            .args(["-setairportpower", device, "on"])
            .output()
            .await
        {
            Ok(output) if output.status.success() => Ok(format!("{} restarted", name)),
            Ok(output) => Err(format!(
                "Failed to re-enable Wi-Fi: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            )),
            Err(e) => Err(format!("Failed to run networksetup: {}", e)),
        }
    } else {
        // Hard toggle via ifconfig
        let down = tokio::process::Command::new("ifconfig")
            .args([device, "down"])
            .output()
            .await;

        if let Ok(output) = &down {
            if !output.status.success() {
                return Err(format!(
                    "Failed to disable {}: {}",
                    name,
                    String::from_utf8_lossy(&output.stderr).trim()
                ));
            }
        }

        tokio::time::sleep(std::time::Duration::from_secs(3)).await;

        match tokio::process::Command::new("ifconfig")
            .args([device, "up"])
            .output()
            .await
        {
            Ok(output) if output.status.success() => Ok(format!("{} restarted", name)),
            Ok(output) => Err(format!(
                "Failed to re-enable {}: {}",
                name,
                String::from_utf8_lossy(&output.stderr).trim()
            )),
            Err(e) => Err(format!("Failed to run ifconfig: {}", e)),
        }
    }
}

#[cfg(target_os = "linux")]
pub async fn restart_adapter(name: &str) -> Result<String, String> {
    // Bring interface down
    let down = tokio::process::Command::new("ip")
        .args(["link", "set", name, "down"])
        .output()
        .await;

    match down {
        Ok(output) if !output.status.success() => {
            return Err(format!(
                "Failed to disable {}: {}",
                name,
                String::from_utf8_lossy(&output.stderr).trim()
            ));
        }
        Err(e) => return Err(format!("Failed to run ip link: {}", e)),
        _ => {}
    }

    tokio::time::sleep(std::time::Duration::from_secs(3)).await;

    // Bring interface up
    match tokio::process::Command::new("ip")
        .args(["link", "set", name, "up"])
        .output()
        .await
    {
        Ok(output) if output.status.success() => Ok(format!("{} restarted", name)),
        Ok(output) => Err(format!(
            "Failed to re-enable {}: {}",
            name,
            String::from_utf8_lossy(&output.stderr).trim()
        )),
        Err(e) => Err(format!("Failed to run ip link: {}", e)),
    }
}

/// Detect the default interface for the current platform.
pub async fn detect_default_interface() -> Option<String> {
    #[cfg(windows)]
    {
        // Use WMI to find the adapter with default route
        let adapters = list_active_adapters().await;
        adapters.into_iter().next()
    }

    #[cfg(target_os = "macos")]
    {
        if let Ok(output) = tokio::process::Command::new("route")
            .args(["-n", "get", "default"])
            .output()
            .await
        {
            let text = String::from_utf8_lossy(&output.stdout);
            for line in text.lines() {
                let line = line.trim();
                if let Some(iface) = line.strip_prefix("interface:") {
                    return Some(iface.trim().to_string());
                }
            }
        }
        None
    }

    #[cfg(target_os = "linux")]
    {
        if let Ok(output) = tokio::process::Command::new("ip")
            .args(["route", "show", "default"])
            .output()
            .await
        {
            let text = String::from_utf8_lossy(&output.stdout);
            // "default via 192.168.1.1 dev eth0 ..."
            for line in text.lines() {
                if let Some(idx) = line.find("dev ") {
                    let after = &line[idx + 4..];
                    if let Some(name) = after.split_whitespace().next() {
                        return Some(name.to_string());
                    }
                }
            }
        }
        None
    }
}

/// Renew DHCP on a specific interface.
pub async fn renew_dhcp_on_interface(iface: &str) -> Result<String, String> {
    #[cfg(windows)]
    {
        let release = tokio::process::Command::new("ipconfig")
            .args(["/release", iface])
            .output()
            .await;
        if let Ok(output) = &release {
            if !output.status.success() {
                // Non-fatal: release may fail if already released
            }
        }
        match tokio::process::Command::new("ipconfig")
            .args(["/renew", iface])
            .output()
            .await
        {
            Ok(output) if output.status.success() => Ok(format!("DHCP renewed on {}", iface)),
            Ok(output) => Err(format!("DHCP renew failed: {}", String::from_utf8_lossy(&output.stderr).trim())),
            Err(e) => Err(format!("Failed to run ipconfig: {}", e)),
        }
    }

    #[cfg(target_os = "macos")]
    {
        match tokio::process::Command::new("ipconfig")
            .args(["set", iface, "DHCP"])
            .output()
            .await
        {
            Ok(output) if output.status.success() => Ok(format!("DHCP renewed on {}", iface)),
            Ok(output) => Err(format!("DHCP renew failed: {}", String::from_utf8_lossy(&output.stderr).trim())),
            Err(e) => Err(format!("Failed to run ipconfig: {}", e)),
        }
    }

    #[cfg(target_os = "linux")]
    {
        // Try NetworkManager first
        if let Ok(output) = tokio::process::Command::new("nmcli")
            .args(["device", "connect", iface])
            .output()
            .await
        {
            if output.status.success() {
                return Ok(format!("DHCP renewed on {} via nmcli", iface));
            }
        }

        // Fallback to dhcpcd
        if let Ok(output) = tokio::process::Command::new("dhcpcd")
            .args(["-n", iface])
            .output()
            .await
        {
            if output.status.success() {
                return Ok(format!("DHCP renewed on {} via dhcpcd", iface));
            }
        }

        // Fallback to dhclient
        let _ = tokio::process::Command::new("dhclient").args(["-r", iface]).output().await;
        match tokio::process::Command::new("dhclient")
            .arg(iface)
            .output()
            .await
        {
            Ok(output) if output.status.success() => Ok(format!("DHCP renewed on {} via dhclient", iface)),
            _ => Err(format!("Could not renew DHCP on {}", iface)),
        }
    }
}
