#[cfg(any(windows, target_os = "linux"))]
use super::cmd::run_owned_mutation_pair;
#[allow(unused_imports)]
use super::cmd::{run_cmd, TIMEOUT_MEDIUM, TIMEOUT_QUICK, TIMEOUT_SLOW};

#[cfg(any(windows, target_os = "linux"))]
fn mutation_succeeded(result: &Result<std::process::Output, String>) -> bool {
    result.as_ref().is_ok_and(|output| output.status.success())
}

#[cfg(any(windows, target_os = "linux"))]
fn mutation_error(result: &Result<std::process::Output, String>) -> String {
    match result {
        Ok(output) => {
            let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
            let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if !stderr.is_empty() {
                stderr
            } else if !stdout.is_empty() {
                stdout
            } else {
                format!("command exited with status {}", output.status)
            }
        }
        Err(error) => error.clone(),
    }
}

/// DHCP lease renewal — all platforms.
pub async fn renew_dhcp() -> Result<String, String> {
    #[cfg(windows)]
    {
        let mut release_cmd = tokio::process::Command::new("ipconfig");
        release_cmd.arg("/release");
        let mut renew_cmd = tokio::process::Command::new("ipconfig");
        renew_cmd.arg("/renew");
        let pair = run_owned_mutation_pair(
            release_cmd,
            TIMEOUT_SLOW,
            std::time::Duration::ZERO,
            renew_cmd,
            TIMEOUT_SLOW,
        )
        .await?;
        if mutation_succeeded(&pair.inverse) {
            Ok("DHCP lease renewed".to_string())
        } else {
            Err(format!(
                "DHCP renew failed: {}",
                mutation_error(&pair.inverse)
            ))
        }
    }

    #[cfg(target_os = "macos")]
    {
        // Detect active network device
        let device = detect_active_device_macos().await;
        match &device {
            Some(dev) => super::adapters::renew_dhcp_on_interface(dev).await,
            None => Err("Could not detect active network device".to_string()),
        }
    }

    #[cfg(target_os = "linux")]
    {
        let mut release_cmd = tokio::process::Command::new("dhclient");
        release_cmd.arg("-r");
        let renew_cmd = tokio::process::Command::new("dhclient");
        let dhclient = run_owned_mutation_pair(
            release_cmd,
            TIMEOUT_SLOW,
            std::time::Duration::ZERO,
            renew_cmd,
            TIMEOUT_SLOW,
        )
        .await?;
        if mutation_succeeded(&dhclient.inverse) {
            return Ok("DHCP lease renewed via dhclient".to_string());
        }

        // Fallback to nmcli
        let mut off_cmd = tokio::process::Command::new("nmcli");
        off_cmd.args(["networking", "off"]);
        let mut on_cmd = tokio::process::Command::new("nmcli");
        on_cmd.args(["networking", "on"]);
        let nmcli = run_owned_mutation_pair(
            off_cmd,
            TIMEOUT_MEDIUM,
            std::time::Duration::ZERO,
            on_cmd,
            TIMEOUT_MEDIUM,
        )
        .await?;
        if !mutation_succeeded(&nmcli.inverse) {
            Err(format!(
                "nmcli networking on failed: {}",
                mutation_error(&nmcli.inverse)
            ))
        } else if mutation_succeeded(&nmcli.first) {
            Ok("DHCP renewed via nmcli".to_string())
        } else {
            Err("Neither dhclient nor nmcli available".to_string())
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
