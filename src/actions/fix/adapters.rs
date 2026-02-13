#[allow(unused_imports)]
use super::cmd::{run_cmd, TIMEOUT_QUICK, TIMEOUT_MEDIUM, TIMEOUT_SLOW};

/// List active network adapters — all platforms.
#[cfg(windows)]
pub async fn list_active_adapters() -> Vec<String> {
    tokio::task::spawn_blocking(|| {
        let adapters = match ipconfig::get_adapters() {
            Ok(a) => a,
            Err(_) => return Vec::new(),
        };

        adapters
            .into_iter()
            .filter(|a| a.oper_status() == ipconfig::OperStatus::IfOperStatusUp)
            .filter(|a| a.if_type() != ipconfig::IfType::SoftwareLoopback)
            .map(|a| a.friendly_name().to_string())
            .collect()
    })
    .await
    .unwrap_or_default()
}

#[cfg(target_os = "macos")]
pub async fn list_active_adapters() -> Vec<String> {
    let mut adapters = Vec::new();
    let mut cmd = tokio::process::Command::new("networksetup");
    cmd.args(["-listallhardwareports"]);
    if let Ok(output) = run_cmd(cmd, TIMEOUT_QUICK).await {
        let text = String::from_utf8_lossy(&output.stdout);
        let mut current_name = String::new();

        for line in text.lines() {
            if let Some(name) = line.strip_prefix("Hardware Port: ") {
                current_name = name.trim().to_string();
            } else if let Some(dev) = line.strip_prefix("Device: ") {
                let device = dev.trim();
                // Check if this interface is active
                let mut ifcfg_cmd = tokio::process::Command::new("ifconfig");
                ifcfg_cmd.arg(device);
                if let Ok(status_output) = run_cmd(ifcfg_cmd, TIMEOUT_QUICK).await {
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
    let mut cmd = tokio::process::Command::new("ip");
    cmd.args(["link", "show", "up"]);
    if let Ok(output) = run_cmd(cmd, TIMEOUT_QUICK).await {
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
    let mut disable_cmd = tokio::process::Command::new("netsh");
    disable_cmd.args(["interface", "set", "interface", name, "disabled"]);
    let disable = run_cmd(disable_cmd, TIMEOUT_SLOW).await;

    match disable {
        Ok(output) if !output.status.success() => {
            return Err(format!(
                "Failed to disable {}: {}",
                name,
                String::from_utf8_lossy(&output.stderr).trim()
            ));
        }
        Err(e) => return Err(e),
        _ => {}
    }

    // Wait for the adapter to fully disable
    tokio::time::sleep(std::time::Duration::from_secs(3)).await;

    // Re-enable
    let mut enable_cmd = tokio::process::Command::new("netsh");
    enable_cmd.args(["interface", "set", "interface", name, "enabled"]);
    match run_cmd(enable_cmd, TIMEOUT_SLOW).await {
        Ok(output) if output.status.success() => Ok(format!("{} restarted", name)),
        Ok(output) => Err(format!(
            "Failed to re-enable {}: {}",
            name,
            String::from_utf8_lossy(&output.stderr).trim()
        )),
        Err(e) => Err(e),
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
        let mut off_cmd = tokio::process::Command::new("networksetup");
        off_cmd.args(["-setairportpower", device, "off"]);
        let off = run_cmd(off_cmd, TIMEOUT_MEDIUM).await;

        if let Ok(output) = &off {
            if !output.status.success() {
                return Err(format!(
                    "Failed to disable Wi-Fi: {}",
                    String::from_utf8_lossy(&output.stderr).trim()
                ));
            }
        }

        tokio::time::sleep(std::time::Duration::from_secs(3)).await;

        let mut on_cmd = tokio::process::Command::new("networksetup");
        on_cmd.args(["-setairportpower", device, "on"]);
        match run_cmd(on_cmd, TIMEOUT_MEDIUM).await {
            Ok(output) if output.status.success() => Ok(format!("{} restarted", name)),
            Ok(output) => Err(format!(
                "Failed to re-enable Wi-Fi: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            )),
            Err(e) => Err(e),
        }
    } else {
        // Hard toggle via ifconfig
        let mut down_cmd = tokio::process::Command::new("ifconfig");
        down_cmd.args([device, "down"]);
        let down = run_cmd(down_cmd, TIMEOUT_MEDIUM).await;

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

        let mut up_cmd = tokio::process::Command::new("ifconfig");
        up_cmd.args([device, "up"]);
        match run_cmd(up_cmd, TIMEOUT_MEDIUM).await {
            Ok(output) if output.status.success() => Ok(format!("{} restarted", name)),
            Ok(output) => Err(format!(
                "Failed to re-enable {}: {}",
                name,
                String::from_utf8_lossy(&output.stderr).trim()
            )),
            Err(e) => Err(e),
        }
    }
}

#[cfg(target_os = "linux")]
pub async fn restart_adapter(name: &str) -> Result<String, String> {
    // Bring interface down
    let mut down_cmd = tokio::process::Command::new("ip");
    down_cmd.args(["link", "set", name, "down"]);
    let down = run_cmd(down_cmd, TIMEOUT_MEDIUM).await;

    match down {
        Ok(output) if !output.status.success() => {
            return Err(format!(
                "Failed to disable {}: {}",
                name,
                String::from_utf8_lossy(&output.stderr).trim()
            ));
        }
        Err(e) => return Err(e),
        _ => {}
    }

    tokio::time::sleep(std::time::Duration::from_secs(3)).await;

    // Bring interface up
    let mut up_cmd = tokio::process::Command::new("ip");
    up_cmd.args(["link", "set", name, "up"]);
    match run_cmd(up_cmd, TIMEOUT_MEDIUM).await {
        Ok(output) if output.status.success() => Ok(format!("{} restarted", name)),
        Ok(output) => Err(format!(
            "Failed to re-enable {}: {}",
            name,
            String::from_utf8_lossy(&output.stderr).trim()
        )),
        Err(e) => Err(e),
    }
}

/// Detect the default interface for the current platform.
pub async fn detect_default_interface() -> Option<String> {
    #[cfg(windows)]
    {
        // Pick the active adapter with lowest IPv4 metric that has a gateway
        tokio::task::spawn_blocking(|| {
            let adapters = match ipconfig::get_adapters() {
                Ok(a) => a,
                Err(_) => return None,
            };

            adapters
                .into_iter()
                .filter(|a| a.oper_status() == ipconfig::OperStatus::IfOperStatusUp)
                .filter(|a| !a.gateways().is_empty())
                .min_by_key(|a| a.ipv4_metric())
                .map(|a| a.friendly_name().to_string())
        })
        .await
        .unwrap_or(None)
    }

    #[cfg(target_os = "macos")]
    {
        let mut cmd = tokio::process::Command::new("route");
        cmd.args(["-n", "get", "default"]);
        if let Ok(output) = run_cmd(cmd, TIMEOUT_QUICK).await {
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
        let mut cmd = tokio::process::Command::new("ip");
        cmd.args(["route", "show", "default"]);
        if let Ok(output) = run_cmd(cmd, TIMEOUT_QUICK).await {
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
        let mut release_cmd = tokio::process::Command::new("ipconfig");
        release_cmd.args(["/release", iface]);
        let release = run_cmd(release_cmd, TIMEOUT_SLOW).await;
        if let Ok(output) = &release {
            if !output.status.success() {
                // Non-fatal: release may fail if already released
            }
        }
        let mut renew_cmd = tokio::process::Command::new("ipconfig");
        renew_cmd.args(["/renew", iface]);
        match run_cmd(renew_cmd, TIMEOUT_SLOW).await {
            Ok(output) if output.status.success() => Ok(format!("DHCP renewed on {}", iface)),
            Ok(output) => Err(format!("DHCP renew failed: {}", String::from_utf8_lossy(&output.stderr).trim())),
            Err(e) => Err(e),
        }
    }

    #[cfg(target_os = "macos")]
    {
        let mut cmd = tokio::process::Command::new("ipconfig");
        cmd.args(["set", iface, "DHCP"]);
        match run_cmd(cmd, TIMEOUT_SLOW).await {
            Ok(output) if output.status.success() => Ok(format!("DHCP renewed on {}", iface)),
            Ok(output) => Err(format!("DHCP renew failed: {}", String::from_utf8_lossy(&output.stderr).trim())),
            Err(e) => Err(e),
        }
    }

    #[cfg(target_os = "linux")]
    {
        // Try NetworkManager first
        let mut nmcli_cmd = tokio::process::Command::new("nmcli");
        nmcli_cmd.args(["device", "connect", iface]);
        if let Ok(output) = run_cmd(nmcli_cmd, TIMEOUT_MEDIUM).await {
            if output.status.success() {
                return Ok(format!("DHCP renewed on {} via nmcli", iface));
            }
        }

        // Fallback to dhcpcd
        let mut dhcpcd_cmd = tokio::process::Command::new("dhcpcd");
        dhcpcd_cmd.args(["-n", iface]);
        if let Ok(output) = run_cmd(dhcpcd_cmd, TIMEOUT_SLOW).await {
            if output.status.success() {
                return Ok(format!("DHCP renewed on {} via dhcpcd", iface));
            }
        }

        // Fallback to dhclient
        let mut release_cmd = tokio::process::Command::new("dhclient");
        release_cmd.args(["-r", iface]);
        let _ = run_cmd(release_cmd, TIMEOUT_SLOW).await;
        let mut renew_cmd = tokio::process::Command::new("dhclient");
        renew_cmd.arg(iface);
        match run_cmd(renew_cmd, TIMEOUT_SLOW).await {
            Ok(output) if output.status.success() => Ok(format!("DHCP renewed on {} via dhclient", iface)),
            _ => Err(format!("Could not renew DHCP on {}", iface)),
        }
    }
}
