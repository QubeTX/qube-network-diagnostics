use crate::config::Config;
use crate::diagnostics::vpn::{self, VpnAdapter};
use crate::render::progress::create_spinner;

use super::cmd::{run_cmd, TIMEOUT_MEDIUM, TIMEOUT_SLOW};
use super::{print_step_ok, print_step_fail, warn_icon};
use crate::actions::{is_interactive, prompt_yes_no};

/// Info about a VPN that was disabled during the fix, for potential re-enable.
pub struct DisabledVpn {
    pub name: String,
    pub method: DisableMethod,
}

pub enum DisableMethod {
    VendorCli(String, Vec<String>), // (binary, args)
    Netsh(String),                  // adapter name (Windows)
    #[cfg(target_os = "macos")]
    Scutil(String),                 // VPN service name
    #[cfg(target_os = "linux")]
    Nmcli(String),                  // connection name
    #[cfg(target_os = "linux")]
    WgQuick(String),                // interface name
}

/// Known enterprise VPN vendors — we never touch these automatically.
fn is_enterprise_vpn(adapter: &VpnAdapter) -> bool {
    let lower = adapter.name.to_lowercase();
    let vendor_lower = adapter.vendor.as_deref().unwrap_or("").to_lowercase();
    let type_lower = adapter.adapter_type.to_lowercase();

    let enterprise_patterns = [
        "cisco", "anyconnect", "globalprotect", "palo alto", "zscaler",
        "forticlient", "fortinet", "pulse secure", "juniper", "f5 ",
        "big-ip", "checkpoint", "corp", "enterprise", "mdm", "company",
    ];

    enterprise_patterns.iter().any(|p| {
        lower.contains(p) || vendor_lower.contains(p) || type_lower.contains(p)
    })
}

/// Try to find a vendor-specific CLI to disconnect this VPN.
fn find_vendor_cli(adapter: &VpnAdapter) -> Option<(String, Vec<String>)> {
    let lower = adapter.name.to_lowercase();
    let vendor_lower = adapter.vendor.as_deref().unwrap_or("").to_lowercase();

    // NordVPN
    if lower.contains("nord") || vendor_lower.contains("nord") {
        return Some(("nordvpn".to_string(), vec!["disconnect".to_string()]));
    }
    // ExpressVPN
    if lower.contains("expressvpn") || vendor_lower.contains("expressvpn") {
        return Some(("expressvpn".to_string(), vec!["disconnect".to_string()]));
    }
    // Mullvad
    if lower.contains("mullvad") || vendor_lower.contains("mullvad") {
        return Some(("mullvad".to_string(), vec!["disconnect".to_string()]));
    }
    // Tailscale
    if lower.contains("tailscale") || vendor_lower.contains("tailscale") {
        return Some(("tailscale".to_string(), vec!["down".to_string()]));
    }
    // WireGuard (wg-quick)
    if adapter.adapter_type == "WireGuard" {
        if let Some(ref iface) = adapter.interface_name {
            return Some(("wg-quick".to_string(), vec!["down".to_string(), iface.clone()]));
        }
    }

    // Cisco AnyConnect
    if lower.contains("cisco") || vendor_lower.contains("cisco") || adapter.adapter_type.contains("Cisco") {
        #[cfg(windows)]
        {
            // Try common install paths
            let paths = [
                r"C:\Program Files (x86)\Cisco\Cisco AnyConnect Secure Mobility Client\vpncli.exe",
                r"C:\Program Files\Cisco\Cisco AnyConnect Secure Mobility Client\vpncli.exe",
            ];
            for path in &paths {
                if std::path::Path::new(path).exists() {
                    return Some((path.to_string(), vec!["disconnect".to_string()]));
                }
            }
        }
        #[cfg(unix)]
        {
            return Some(("/opt/cisco/anyconnect/bin/vpn".to_string(), vec!["disconnect".to_string()]));
        }
    }

    None
}

/// Detect connected VPNs and prompt user to disable them before fix stages.
/// Returns a list of VPNs that were disabled (for later re-enable).
pub async fn detect_and_disable(config: &Config) -> Vec<DisabledVpn> {
    let mut disabled = Vec::new();

    let spinner = create_spinner("Detecting VPN connections...");
    let vpns = vpn::collect().await;
    spinner.finish_and_clear();

    let vpns = match vpns {
        Some(v) => v,
        None => return disabled,
    };

    let connected: Vec<&VpnAdapter> = vpns.iter().filter(|v| v.status == "Connected").collect();
    if connected.is_empty() {
        return disabled;
    }

    for adapter in connected {
        if is_enterprise_vpn(adapter) {
            if is_interactive(config) {
                println!(
                    "  {} Corporate VPN detected: {} — skipping (managed by your organization)",
                    warn_icon(config),
                    crate::render::color::cyan(&adapter.name, config),
                );
            }
            continue;
        }

        let do_disable = if is_interactive(config) {
            let prompt = format!(
                "  VPN detected: {} ({}). VPN connections can interfere with network fixes. Disable? (y/N): ",
                adapter.name, adapter.adapter_type,
            );
            prompt_yes_no(&prompt)
        } else {
            // JSON mode: auto-disable consumer VPNs
            true
        };

        if !do_disable {
            continue;
        }

        // Try vendor CLI first
        if let Some((bin, args)) = find_vendor_cli(adapter) {
            let spinner = create_spinner(&format!("Disabling {}...", adapter.name));
            let mut cmd = tokio::process::Command::new(&bin);
            cmd.args(&args);
            let result = run_cmd(cmd, TIMEOUT_MEDIUM).await;
            spinner.finish_and_clear();

            if let Ok(output) = result {
                if output.status.success() {
                    if is_interactive(config) {
                        print_step_ok(&format!("Disabled {}", adapter.name), config);
                    }
                    disabled.push(DisabledVpn {
                        name: adapter.name.clone(),
                        method: DisableMethod::VendorCli(bin, args.iter().map(|a| a.replace("disconnect", "connect").replace("down", "up")).collect()),
                    });
                    continue;
                }
            }
        }

        // Fallback: platform-specific adapter disable
        let spinner = create_spinner(&format!("Disabling {}...", adapter.name));
        let fallback_result = disable_adapter_fallback(adapter, config).await;
        spinner.finish_and_clear();
        match fallback_result {
            Some(d) => disabled.push(d),
            None => {
                if is_interactive(config) {
                    print_step_fail(
                        &format!("Could not disable {}", adapter.name),
                        "Try disconnecting manually before running fix",
                        config,
                    );
                }
            }
        }
    }

    if !disabled.is_empty() {
        // Give VPNs time to fully disconnect
        let spinner = create_spinner("Waiting for VPN disconnect...");
        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
        spinner.finish_and_clear();
    }

    disabled
}

async fn disable_adapter_fallback(adapter: &VpnAdapter, config: &Config) -> Option<DisabledVpn> {
    #[cfg(windows)]
    {
        // Try netsh to disable the adapter
        if let Some(ref iface) = adapter.interface_name {
            let mut cmd = tokio::process::Command::new("netsh");
            cmd.args(["interface", "set", "interface", iface, "disabled"]);
            if let Ok(output) = run_cmd(cmd, TIMEOUT_SLOW).await {
                if output.status.success() {
                    if is_interactive(config) {
                        print_step_ok(&format!("Disabled {}", adapter.name), config);
                    }
                    return Some(DisabledVpn {
                        name: adapter.name.clone(),
                        method: DisableMethod::Netsh(iface.clone()),
                    });
                }
            }
        }
        let _ = config;
        None
    }

    #[cfg(target_os = "macos")]
    {
        // Try scutil --nc stop
        let mut cmd = tokio::process::Command::new("scutil");
        cmd.args(["--nc", "stop", &adapter.name]);
        if let Ok(output) = run_cmd(cmd, TIMEOUT_MEDIUM).await {
            if output.status.success() {
                if is_interactive(config) {
                    print_step_ok(&format!("Disabled {}", adapter.name), config);
                }
                return Some(DisabledVpn {
                    name: adapter.name.clone(),
                    method: DisableMethod::Scutil(adapter.name.clone()),
                });
            }
        }
        None
    }

    #[cfg(target_os = "linux")]
    {
        // Try nmcli connection down
        let mut nmcli_cmd = tokio::process::Command::new("nmcli");
        nmcli_cmd.args(["connection", "down", &adapter.name]);
        if let Ok(output) = run_cmd(nmcli_cmd, TIMEOUT_MEDIUM).await {
            if output.status.success() {
                if is_interactive(config) {
                    print_step_ok(&format!("Disabled {}", adapter.name), config);
                }
                return Some(DisabledVpn {
                    name: adapter.name.clone(),
                    method: DisableMethod::Nmcli(adapter.name.clone()),
                });
            }
        }
        // Try wg-quick down
        if let Some(ref iface) = adapter.interface_name {
            let mut wg_cmd = tokio::process::Command::new("wg-quick");
            wg_cmd.args(["down", iface]);
            if let Ok(output) = run_cmd(wg_cmd, TIMEOUT_MEDIUM).await {
                if output.status.success() {
                    if is_interactive(config) {
                        print_step_ok(&format!("Disabled {}", adapter.name), config);
                    }
                    return Some(DisabledVpn {
                        name: adapter.name.clone(),
                        method: DisableMethod::WgQuick(iface.clone()),
                    });
                }
            }
        }
        let _ = config;
        None
    }
}

/// Offer to re-enable VPNs that were disabled during the fix.
pub async fn offer_reenable(disabled: &[DisabledVpn], config: &Config) {
    if disabled.is_empty() {
        return;
    }

    for vpn in disabled {
        let do_reenable = if is_interactive(config) {
            let prompt = format!("  Re-enable {}? (y/N): ", vpn.name);
            prompt_yes_no(&prompt)
        } else {
            // JSON mode: skip re-enable
            false
        };

        if !do_reenable {
            continue;
        }

        let spinner = create_spinner(&format!("Re-enabling {}...", vpn.name));
        let success = reenable_vpn(vpn).await;
        spinner.finish_and_clear();

        if success {
            if is_interactive(config) {
                print_step_ok(&format!("Re-enabled {}", vpn.name), config);
            }
            // Verify connectivity after re-enable
            let spinner = create_spinner("Verifying connectivity...");
            tokio::time::sleep(std::time::Duration::from_secs(5)).await;
            let connected = super::connectivity::check_connectivity().await;
            spinner.finish_and_clear();

            if !connected {
                // Auto-disable again
                let spinner = create_spinner(&format!("Disabling {} again...", vpn.name));
                let _ = redisable_vpn(vpn).await;
                spinner.finish_and_clear();

                if is_interactive(config) {
                    println!(
                        "  {} Re-enabling {} broke connectivity. The VPN has been disabled again.",
                        warn_icon(config),
                        crate::render::color::cyan(&vpn.name, config),
                    );
                    println!(
                        "    {}",
                        crate::render::color::dim("Check your VPN configuration or contact your VPN provider.", config),
                    );
                }
            }
        } else if is_interactive(config) {
            print_step_fail(&format!("Failed to re-enable {}", vpn.name), "Try reconnecting manually", config);
        }
    }
}

async fn reenable_vpn(vpn: &DisabledVpn) -> bool {
    match &vpn.method {
        DisableMethod::VendorCli(bin, reconnect_args) => {
            let mut cmd = tokio::process::Command::new(bin);
            cmd.args(reconnect_args);
            if let Ok(output) = run_cmd(cmd, TIMEOUT_MEDIUM).await {
                return output.status.success();
            }
            false
        }
        DisableMethod::Netsh(iface) => {
            let mut cmd = tokio::process::Command::new("netsh");
            cmd.args(["interface", "set", "interface", iface, "enabled"]);
            if let Ok(output) = run_cmd(cmd, TIMEOUT_SLOW).await {
                return output.status.success();
            }
            false
        }
        #[cfg(target_os = "macos")]
        DisableMethod::Scutil(service) => {
            let mut cmd = tokio::process::Command::new("scutil");
            cmd.args(["--nc", "start", service]);
            if let Ok(output) = run_cmd(cmd, TIMEOUT_MEDIUM).await {
                return output.status.success();
            }
            false
        }
        #[cfg(target_os = "linux")]
        DisableMethod::Nmcli(conn) => {
            let mut cmd = tokio::process::Command::new("nmcli");
            cmd.args(["connection", "up", conn]);
            if let Ok(output) = run_cmd(cmd, TIMEOUT_MEDIUM).await {
                return output.status.success();
            }
            false
        }
        #[cfg(target_os = "linux")]
        DisableMethod::WgQuick(iface) => {
            let mut cmd = tokio::process::Command::new("wg-quick");
            cmd.args(["up", iface]);
            if let Ok(output) = run_cmd(cmd, TIMEOUT_MEDIUM).await {
                return output.status.success();
            }
            false
        }
    }
}

async fn redisable_vpn(vpn: &DisabledVpn) -> bool {
    match &vpn.method {
        DisableMethod::VendorCli(bin, reconnect_args) => {
            let disconnect_args: Vec<String> = reconnect_args.iter()
                .map(|a| a.replace("connect", "disconnect").replace("up", "down"))
                .collect();
            let mut cmd = tokio::process::Command::new(bin);
            cmd.args(&disconnect_args);
            if let Ok(output) = run_cmd(cmd, TIMEOUT_MEDIUM).await {
                return output.status.success();
            }
            false
        }
        DisableMethod::Netsh(iface) => {
            let mut cmd = tokio::process::Command::new("netsh");
            cmd.args(["interface", "set", "interface", iface, "disabled"]);
            if let Ok(output) = run_cmd(cmd, TIMEOUT_SLOW).await {
                return output.status.success();
            }
            false
        }
        #[cfg(target_os = "macos")]
        DisableMethod::Scutil(service) => {
            let mut cmd = tokio::process::Command::new("scutil");
            cmd.args(["--nc", "stop", service]);
            if let Ok(output) = run_cmd(cmd, TIMEOUT_MEDIUM).await {
                return output.status.success();
            }
            false
        }
        #[cfg(target_os = "linux")]
        DisableMethod::Nmcli(conn) => {
            let mut cmd = tokio::process::Command::new("nmcli");
            cmd.args(["connection", "down", conn]);
            if let Ok(output) = run_cmd(cmd, TIMEOUT_MEDIUM).await {
                return output.status.success();
            }
            false
        }
        #[cfg(target_os = "linux")]
        DisableMethod::WgQuick(iface) => {
            let mut cmd = tokio::process::Command::new("wg-quick");
            cmd.args(["down", iface]);
            if let Ok(output) = run_cmd(cmd, TIMEOUT_MEDIUM).await {
                return output.status.success();
            }
            false
        }
    }
}

/// Serialize VPN state for JSON output.
pub fn vpn_json(disabled: &[DisabledVpn]) -> serde_json::Value {
    if disabled.is_empty() {
        return serde_json::json!(null);
    }
    let items: Vec<serde_json::Value> = disabled.iter().map(|v| {
        serde_json::json!({
            "name": v.name,
            "disabled": true,
        })
    }).collect();
    serde_json::json!(items)
}
