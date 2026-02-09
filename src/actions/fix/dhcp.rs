/// DHCP lease renewal — all platforms.
pub async fn renew_dhcp() -> Result<String, String> {
    #[cfg(windows)]
    {
        // Release first, then renew
        let release = tokio::process::Command::new("ipconfig")
            .arg("/release")
            .output()
            .await;

        if let Ok(output) = &release {
            if !output.status.success() {
                let msg = String::from_utf8_lossy(&output.stderr).trim().to_string();
                if !msg.is_empty() {
                    return Err(format!("DHCP release failed: {}", msg));
                }
            }
        }

        match tokio::process::Command::new("ipconfig")
            .arg("/renew")
            .output()
            .await
        {
            Ok(output) if output.status.success() => {
                Ok("DHCP lease renewed".to_string())
            }
            Ok(output) => Err(format!(
                "DHCP renew failed: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            )),
            Err(e) => Err(format!("Failed to run ipconfig: {}", e)),
        }
    }

    #[cfg(target_os = "macos")]
    {
        // Detect active network device
        let device = detect_active_device_macos().await;
        match &device {
            Some(dev) => {
                match tokio::process::Command::new("ipconfig")
                    .args(["set", dev, "DHCP"])
                    .output()
                    .await
                {
                    Ok(output) if output.status.success() => {
                        Ok(format!("DHCP renewed on {}", dev))
                    }
                    Ok(output) => Err(format!(
                        "DHCP renew failed on {}: {}",
                        dev,
                        String::from_utf8_lossy(&output.stderr).trim()
                    )),
                    Err(e) => Err(format!("Failed to run ipconfig: {}", e)),
                }
            }
            None => Err("Could not detect active network device".to_string()),
        }
    }

    #[cfg(target_os = "linux")]
    {
        // Try dhclient first
        let dhclient_release = tokio::process::Command::new("dhclient")
            .arg("-r")
            .output()
            .await;

        if let Ok(output) = dhclient_release {
            if output.status.success() {
                match tokio::process::Command::new("dhclient")
                    .output()
                    .await
                {
                    Ok(output) if output.status.success() => {
                        return Ok("DHCP lease renewed via dhclient".to_string());
                    }
                    _ => {}
                }
            }
        }

        // Fallback to nmcli
        match tokio::process::Command::new("nmcli")
            .args(["networking", "off"])
            .output()
            .await
        {
            Ok(off_output) if off_output.status.success() => {
                match tokio::process::Command::new("nmcli")
                    .args(["networking", "on"])
                    .output()
                    .await
                {
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
