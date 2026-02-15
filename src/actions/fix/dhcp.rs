#[allow(unused_imports)]
use super::cmd::{run_cmd, TIMEOUT_QUICK, TIMEOUT_MEDIUM, TIMEOUT_SLOW};

/// DHCP lease renewal — all platforms.
pub async fn renew_dhcp() -> Result<String, String> {
    #[cfg(windows)]
    {
        // Release first, then renew
        let mut release_cmd = tokio::process::Command::new("ipconfig");
        release_cmd.arg("/release");
        let release = run_cmd(release_cmd, TIMEOUT_SLOW).await;

        if let Ok(output) = &release {
            if !output.status.success() {
                let msg = String::from_utf8_lossy(&output.stderr).trim().to_string();
                if !msg.is_empty() {
                    return Err(format!("DHCP release failed: {}", msg));
                }
            }
        }

        let mut renew_cmd = tokio::process::Command::new("ipconfig");
        renew_cmd.arg("/renew");
        match run_cmd(renew_cmd, TIMEOUT_SLOW).await {
            Ok(output) if output.status.success() => {
                Ok("DHCP lease renewed".to_string())
            }
            Ok(output) => Err(format!(
                "DHCP renew failed: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            )),
            Err(e) => Err(e),
        }
    }

    #[cfg(target_os = "macos")]
    {
        // Detect active network device
        let device = detect_active_device_macos().await;
        match &device {
            Some(dev) => {
                let mut cmd = tokio::process::Command::new("ipconfig");
                cmd.args(["set", dev, "DHCP"]);
                match run_cmd(cmd, TIMEOUT_SLOW).await {
                    Ok(output) if output.status.success() => {
                        Ok(format!("DHCP renewed on {}", dev))
                    }
                    Ok(output) => Err(format!(
                        "DHCP renew failed on {}: {}",
                        dev,
                        String::from_utf8_lossy(&output.stderr).trim()
                    )),
                    Err(e) => Err(e),
                }
            }
            None => Err("Could not detect active network device".to_string()),
        }
    }

    #[cfg(target_os = "linux")]
    {
        // Try dhclient first
        let mut release_cmd = tokio::process::Command::new("dhclient");
        release_cmd.arg("-r");
        let dhclient_release = run_cmd(release_cmd, TIMEOUT_SLOW).await;

        if let Ok(output) = dhclient_release {
            if output.status.success() {
                let renew_cmd = tokio::process::Command::new("dhclient");
                match run_cmd(renew_cmd, TIMEOUT_SLOW).await {
                    Ok(output) if output.status.success() => {
                        return Ok("DHCP lease renewed via dhclient".to_string());
                    }
                    _ => {}
                }
            }
        }

        // Fallback to nmcli
        let mut off_cmd = tokio::process::Command::new("nmcli");
        off_cmd.args(["networking", "off"]);
        match run_cmd(off_cmd, TIMEOUT_MEDIUM).await {
            Ok(off_output) if off_output.status.success() => {
                let mut on_cmd = tokio::process::Command::new("nmcli");
                on_cmd.args(["networking", "on"]);
                match run_cmd(on_cmd, TIMEOUT_MEDIUM).await {
                    Ok(on_output) if on_output.status.success() => {
                        Ok("DHCP renewed via nmcli".to_string())
                    }
                    _ => Err("nmcli networking on failed".to_string()),
                }
            }
            Ok(_) => Err("Neither dhclient nor nmcli available".to_string()),
            Err(_) => Err("Neither dhclient nor nmcli available".to_string()),
        }
    }
}

/// Detect the active network device on macOS using `route -n get default`.
#[cfg(target_os = "macos")]
async fn detect_active_device_macos() -> Option<String> {
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
