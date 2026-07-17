#[allow(unused_imports)]
use super::cmd::{run_cmd, TIMEOUT_MEDIUM, TIMEOUT_QUICK, TIMEOUT_SLOW};
#[cfg(any(windows, target_os = "linux"))]
use super::cmd::{run_owned_mutation, run_owned_mutation_pair, MutationPairOutput};

#[cfg(any(windows, target_os = "linux"))]
fn mutation_failure(result: &Result<std::process::Output, String>) -> Option<String> {
    match result {
        Ok(output) if output.status.success() => None,
        Ok(output) => {
            let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
            let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
            Some(if !stderr.is_empty() {
                stderr
            } else if !stdout.is_empty() {
                stdout
            } else {
                format!("command exited with status {}", output.status)
            })
        }
        Err(error) => Some(error.clone()),
    }
}

#[cfg(any(windows, target_os = "linux"))]
fn require_inverse(pair: &MutationPairOutput, label: &str) -> Result<(), String> {
    mutation_failure(&pair.inverse).map_or(Ok(()), |error| Err(format!("{label}: {error}")))
}

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
                    if name != "lo"
                        && !name.starts_with("veth")
                        && !name.starts_with("docker")
                        && !name.starts_with("br-")
                    {
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
    let mut disable_cmd = tokio::process::Command::new("netsh");
    disable_cmd.args(["interface", "set", "interface", name, "disabled"]);
    let mut enable_cmd = tokio::process::Command::new("netsh");
    enable_cmd.args(["interface", "set", "interface", name, "enabled"]);
    let pair = run_owned_mutation_pair(
        disable_cmd,
        TIMEOUT_SLOW,
        std::time::Duration::from_secs(3),
        enable_cmd,
        TIMEOUT_SLOW,
    )
    .await?;

    require_inverse(&pair, &format!("Failed to re-enable {name}"))?;
    if let Some(error) = mutation_failure(&pair.first) {
        Err(format!(
            "Failed to disable {name}: {error}; conservative re-enable completed"
        ))
    } else {
        Ok(format!("{} restarted", name))
    }
}

#[cfg(target_os = "macos")]
pub async fn restart_adapter(name_and_device: &str) -> Result<String, String> {
    // This legacy helper cannot participate in the interrupt restore registry.
    // Keep the API but fail closed; the fix loop's guarded BounceInterface
    // action is the only permitted macOS adapter-cycle path.
    Err(format!(
        "Refused unguarded macOS adapter restart for {}. Use `nd300 fix`, which registers re-enable cleanup before disconnecting the interface.",
        name_and_device
    ))
}
#[cfg(target_os = "linux")]
pub async fn restart_adapter(name: &str) -> Result<String, String> {
    let mut down_cmd = tokio::process::Command::new("ip");
    down_cmd.args(["link", "set", name, "down"]);
    let mut up_cmd = tokio::process::Command::new("ip");
    up_cmd.args(["link", "set", name, "up"]);
    let pair = run_owned_mutation_pair(
        down_cmd,
        TIMEOUT_MEDIUM,
        std::time::Duration::from_secs(3),
        up_cmd,
        TIMEOUT_MEDIUM,
    )
    .await?;

    require_inverse(&pair, &format!("Failed to re-enable {name}"))?;
    if let Some(error) = mutation_failure(&pair.first) {
        Err(format!(
            "Failed to disable {name}: {error}; conservative re-enable completed"
        ))
    } else {
        Ok(format!("{} restarted", name))
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
#[cfg(any(target_os = "macos", test))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MacosIpv4Mode {
    Dhcp,
    Manual,
    Bootp,
    Unknown,
}

#[cfg(any(target_os = "macos", test))]
fn parse_macos_ipv4_mode(text: &str) -> MacosIpv4Mode {
    if text.lines().any(|line| line.trim() == "DHCP Configuration") {
        MacosIpv4Mode::Dhcp
    } else if text
        .lines()
        .any(|line| line.trim() == "Manual Configuration")
    {
        MacosIpv4Mode::Manual
    } else if text
        .lines()
        .any(|line| line.trim().eq_ignore_ascii_case("BOOTP Configuration"))
    {
        MacosIpv4Mode::Bootp
    } else {
        MacosIpv4Mode::Unknown
    }
}

#[cfg(any(target_os = "macos", test))]
fn require_macos_dhcp_mode(mode: MacosIpv4Mode, service: &str) -> Result<(), String> {
    match mode {
        MacosIpv4Mode::Dhcp => Ok(()),
        MacosIpv4Mode::Manual => Err(format!(
            "Skipped DHCP renewal on {}: the service uses a manual/static IPv4 configuration",
            service
        )),
        MacosIpv4Mode::Bootp => Err(format!(
            "Skipped DHCP renewal on {}: the service uses BOOTP",
            service
        )),
        MacosIpv4Mode::Unknown => Err(format!(
            "Refused DHCP renewal on {} because its IPv4 mode could not be verified as DHCP",
            service
        )),
    }
}

pub async fn renew_dhcp_on_interface(iface: &str) -> Result<String, String> {
    #[cfg(windows)]
    {
        let mut release_cmd = tokio::process::Command::new("ipconfig");
        release_cmd.args(["/release", iface]);
        let mut renew_cmd = tokio::process::Command::new("ipconfig");
        renew_cmd.args(["/renew", iface]);
        let pair = run_owned_mutation_pair(
            release_cmd,
            TIMEOUT_SLOW,
            std::time::Duration::ZERO,
            renew_cmd,
            TIMEOUT_SLOW,
        )
        .await?;
        require_inverse(&pair, "DHCP renew failed")?;
        Ok(format!("DHCP renewed on {}", iface))
    }

    #[cfg(target_os = "macos")]
    {
        let service = super::stages::detect_macos_service(iface)
            .await
            .ok_or_else(|| {
                format!(
                    "Refused DHCP renewal: no unambiguous macOS network service maps to {}",
                    iface
                )
            })?;
        let mut inspect = tokio::process::Command::new("networksetup");
        inspect.args(["-getinfo", &service]);
        let inspect_output = run_cmd(inspect, TIMEOUT_QUICK).await?;
        if !inspect_output.status.success() {
            return Err(format!(
                "Refused DHCP renewal: could not inspect IPv4 mode for {}",
                service
            ));
        }
        require_macos_dhcp_mode(
            parse_macos_ipv4_mode(&String::from_utf8_lossy(&inspect_output.stdout)),
            &service,
        )?;

        let mut cmd = tokio::process::Command::new("ipconfig");
        cmd.args(["set", iface, "DHCP"]);
        match super::cmd::run_macos_mutation(cmd, TIMEOUT_SLOW).await {
            Ok(output) if output.status.success() => Ok(format!("DHCP renewed on {}", iface)),
            Ok(output) => Err(format!(
                "DHCP renew failed: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            )),
            Err(e) => Err(e),
        }
    }

    #[cfg(target_os = "linux")]
    {
        // Try NetworkManager first
        let mut nmcli_cmd = tokio::process::Command::new("nmcli");
        nmcli_cmd.args(["device", "connect", iface]);
        if let Ok(output) = run_owned_mutation(nmcli_cmd, TIMEOUT_MEDIUM).await {
            if output.status.success() {
                return Ok(format!("DHCP renewed on {} via nmcli", iface));
            }
        }

        // Fallback to dhcpcd
        let mut dhcpcd_cmd = tokio::process::Command::new("dhcpcd");
        dhcpcd_cmd.args(["-n", iface]);
        if let Ok(output) = run_owned_mutation(dhcpcd_cmd, TIMEOUT_SLOW).await {
            if output.status.success() {
                return Ok(format!("DHCP renewed on {} via dhcpcd", iface));
            }
        }

        // Fallback to dhclient
        let mut release_cmd = tokio::process::Command::new("dhclient");
        release_cmd.args(["-r", iface]);
        let mut renew_cmd = tokio::process::Command::new("dhclient");
        renew_cmd.arg(iface);
        let pair = run_owned_mutation_pair(
            release_cmd,
            TIMEOUT_SLOW,
            std::time::Duration::ZERO,
            renew_cmd,
            TIMEOUT_SLOW,
        )
        .await?;
        if mutation_failure(&pair.inverse).is_none() {
            Ok(format!("DHCP renewed on {} via dhclient", iface))
        } else {
            Err(format!("Could not renew DHCP on {}", iface))
        }
    }
}

#[cfg(test)]
mod macos_dhcp_safety_tests {
    use super::*;

    #[test]
    fn classifies_dhcp_manual_bootp_and_unknown_without_guessing() {
        assert_eq!(
            parse_macos_ipv4_mode("DHCP Configuration\nIP address: 10.0.0.2\n"),
            MacosIpv4Mode::Dhcp
        );
        assert_eq!(
            parse_macos_ipv4_mode("Manual Configuration\nIP address: 10.0.0.2\n"),
            MacosIpv4Mode::Manual
        );
        assert_eq!(
            parse_macos_ipv4_mode("BOOTP Configuration\n"),
            MacosIpv4Mode::Bootp
        );
        assert_eq!(
            parse_macos_ipv4_mode("IPv6: Automatic\n"),
            MacosIpv4Mode::Unknown
        );
        assert!(require_macos_dhcp_mode(MacosIpv4Mode::Dhcp, "Wi-Fi").is_ok());
        assert!(require_macos_dhcp_mode(MacosIpv4Mode::Manual, "Wi-Fi")
            .unwrap_err()
            .contains("manual/static"));
        assert!(require_macos_dhcp_mode(MacosIpv4Mode::Bootp, "Wi-Fi").is_err());
        assert!(require_macos_dhcp_mode(MacosIpv4Mode::Unknown, "Wi-Fi").is_err());
    }
}
