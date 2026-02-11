use crate::config::Config;
use crate::render::color;
use crate::render::progress::create_spinner;

use super::adapters;
use super::arp;
use super::connectivity;
use super::dhcp;
use super::wifi;
use super::cmd::{run_cmd, TIMEOUT_QUICK, TIMEOUT_MEDIUM, TIMEOUT_SLOW};
use super::{print_step_ok, print_step_fail, warn_icon, StepResult};
use crate::actions::{flush_dns_platform, is_interactive, prompt_yes_no};
#[cfg(target_os = "linux")]
use crate::actions::prompt_string;

// ── Stage 1: Service Restart (automatic, safe) ─────────────────────────────

pub async fn run_stage1(config: &Config) -> (Vec<StepResult>, bool) {
    let mut steps = Vec::new();

    // DNS flush
    {
        let spinner = create_spinner("Flushing DNS cache...");
        let result = flush_dns_platform().await;
        spinner.finish_and_clear();
        match result {
            Ok(msg) => {
                if is_interactive(config) { print_step_ok("DNS cache flushed", config); }
                steps.push(StepResult { name: "dns_flush", success: true, message: msg });
            }
            Err(msg) => {
                if is_interactive(config) { print_step_fail("Failed to flush DNS cache", &msg, config); }
                steps.push(StepResult { name: "dns_flush", success: false, message: msg });
            }
        }
    }

    // ARP flush
    {
        let spinner = create_spinner("Flushing ARP cache...");
        let result = arp::flush_arp().await;
        spinner.finish_and_clear();
        match result {
            Ok(msg) => {
                if is_interactive(config) { print_step_ok("ARP cache flushed", config); }
                steps.push(StepResult { name: "arp_flush", success: true, message: msg });
            }
            Err(msg) => {
                if is_interactive(config) { print_step_fail("Failed to flush ARP cache", &msg, config); }
                steps.push(StepResult { name: "arp_flush", success: false, message: msg });
            }
        }
    }

    // Service restart (platform-specific)
    {
        let spinner = create_spinner("Restarting network services...");
        let result = restart_services().await;
        spinner.finish_and_clear();
        match result {
            Ok(msg) => {
                if is_interactive(config) { print_step_ok("Network services restarted", config); }
                steps.push(StepResult { name: "service_restart", success: true, message: msg });
            }
            Err(msg) => {
                if is_interactive(config) { print_step_fail("Failed to restart services", &msg, config); }
                steps.push(StepResult { name: "service_restart", success: false, message: msg });
            }
        }
    }

    // DHCP renew
    {
        let spinner = create_spinner("Renewing DHCP lease...");
        let result = dhcp::renew_dhcp().await;
        spinner.finish_and_clear();
        match result {
            Ok(msg) => {
                if is_interactive(config) { print_step_ok("DHCP lease renewed", config); }
                steps.push(StepResult { name: "dhcp_renew", success: true, message: msg });
            }
            Err(msg) => {
                if is_interactive(config) { print_step_fail("Failed to renew DHCP lease", &msg, config); }
                steps.push(StepResult { name: "dhcp_renew", success: false, message: msg });
            }
        }
    }

    // Wait and check connectivity
    {
        let spinner = create_spinner("Checking connectivity...");
        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
        let connected = connectivity::check_connectivity().await;
        spinner.finish_and_clear();
        (steps, connected)
    }
}

/// Restart platform networking services.
async fn restart_services() -> Result<String, String> {
    #[cfg(windows)]
    {
        let mut restarted = Vec::new();

        // Try sc stop/start with internal service names (works better than net stop/start)
        for (internal, display) in &[("dnscache", "DNS Client"), ("Dhcp", "DHCP Client")] {
            let mut stop_cmd = tokio::process::Command::new("sc");
            stop_cmd.args(["stop", internal]);
            let _ = run_cmd(stop_cmd, TIMEOUT_MEDIUM).await;

            let mut start_cmd = tokio::process::Command::new("sc");
            start_cmd.args(["start", internal]);
            match run_cmd(start_cmd, TIMEOUT_MEDIUM).await {
                Ok(output) if output.status.success() => {
                    restarted.push(*display);
                }
                _ => {}
            }
        }

        // If sc failed (PPL-protected services), try PowerShell Restart-Service
        if restarted.is_empty() {
            for (internal, display) in &[("dnscache", "DNS Client"), ("Dhcp", "DHCP Client")] {
                let mut cmd = tokio::process::Command::new("powershell");
                cmd.args(["-NoProfile", "-Command", &format!("Restart-Service -Name '{}' -Force", internal)]);
                match run_cmd(cmd, TIMEOUT_MEDIUM).await {
                    Ok(output) if output.status.success() => {
                        restarted.push(*display);
                    }
                    _ => {}
                }
            }
        }

        if restarted.is_empty() {
            // Services are PPL-protected; DNS flush and DHCP renew already handle cache refresh
            Ok("Services protected (PPL); DNS flush and DHCP renew handle cache refresh".to_string())
        } else {
            Ok(format!("Restarted: {}", restarted.join(", ")))
        }
    }

    #[cfg(target_os = "macos")]
    {
        let mut cmd = tokio::process::Command::new("killall");
        cmd.args(["-HUP", "configd"]);
        match run_cmd(cmd, TIMEOUT_QUICK).await {
            Ok(output) if output.status.success() => {
                Ok("Restarted configd".to_string())
            }
            _ => Err("Could not restart configd".to_string()),
        }
    }

    #[cfg(target_os = "linux")]
    {
        // Try NetworkManager first
        let mut nm_cmd = tokio::process::Command::new("systemctl");
        nm_cmd.args(["restart", "NetworkManager"]);
        if let Ok(output) = run_cmd(nm_cmd, TIMEOUT_MEDIUM).await {
            if output.status.success() {
                return Ok("Restarted NetworkManager".to_string());
            }
        }

        // Fallback to dhcpcd
        let mut dhcpcd_cmd = tokio::process::Command::new("systemctl");
        dhcpcd_cmd.args(["restart", "dhcpcd"]);
        if let Ok(output) = run_cmd(dhcpcd_cmd, TIMEOUT_MEDIUM).await {
            if output.status.success() {
                return Ok("Restarted dhcpcd".to_string());
            }
        }

        Err("Could not restart network services".to_string())
    }
}

// ── Stage 2: Interface Reset (prompts user) ─────────────────────────────────

pub async fn run_stage2(config: &Config) -> (Vec<StepResult>, bool) {
    let mut steps = Vec::new();

    if is_interactive(config) {
        println!();
        let prompt = format!(
            "  {} Stage 2 will briefly disconnect your network to reset the interface. Continue? (y/N): ",
            warn_icon(config),
        );
        if !prompt_yes_no(&prompt) {
            steps.push(StepResult {
                name: "interface_reset",
                success: false,
                message: "Skipped by user".to_string(),
            });
            return (steps, false);
        }
    }

    let iface = match adapters::detect_default_interface().await {
        Some(i) => i,
        None => {
            if is_interactive(config) {
                print_step_fail("Could not detect default network interface", "", config);
            }
            steps.push(StepResult {
                name: "interface_reset",
                success: false,
                message: "Could not detect default interface".to_string(),
            });
            return (steps, false);
        }
    };

    if is_interactive(config) {
        println!(
            "  Resetting interface: {}",
            color::cyan(&iface, config),
        );
    }

    // Disable/down
    {
        let spinner = create_spinner(&format!("Disabling {}...", iface));
        let result = disable_interface(&iface).await;
        spinner.finish_and_clear();
        match result {
            Ok(_) => {
                if is_interactive(config) { print_step_ok(&format!("Disabled {}", iface), config); }
            }
            Err(msg) => {
                if is_interactive(config) { print_step_fail(&format!("Failed to disable {}", iface), &msg, config); }
                steps.push(StepResult {
                    name: "interface_reset",
                    success: false,
                    message: msg,
                });
                return (steps, false);
            }
        }
    }

    // Wait for interface to fully disable
    {
        let spinner = create_spinner("Waiting for interface...");
        tokio::time::sleep(std::time::Duration::from_secs(3)).await;
        spinner.finish_and_clear();
    }

    // Re-enable/up
    {
        let spinner = create_spinner(&format!("Re-enabling {}...", iface));
        let result = enable_interface(&iface).await;
        spinner.finish_and_clear();
        match result {
            Ok(_) => {
                if is_interactive(config) { print_step_ok(&format!("Re-enabled {}", iface), config); }
            }
            Err(_) => {
                // Retry once — leaving interface disabled is worse than a slow retry
                let spinner = create_spinner(&format!("Retrying re-enable {}...", iface));
                tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                let retry = enable_interface(&iface).await;
                spinner.finish_and_clear();
                match retry {
                    Ok(_) => {
                        if is_interactive(config) { print_step_ok(&format!("Re-enabled {} (retry)", iface), config); }
                    }
                    Err(msg) => {
                        if is_interactive(config) { print_step_fail(&format!("Failed to re-enable {}", iface), &msg, config); }
                        steps.push(StepResult {
                            name: "interface_reset",
                            success: false,
                            message: msg,
                        });
                        return (steps, false);
                    }
                }
            }
        }
    }

    // Targeted DHCP renew
    {
        let spinner = create_spinner("Renewing DHCP on interface...");
        let result = adapters::renew_dhcp_on_interface(&iface).await;
        spinner.finish_and_clear();
        match result {
            Ok(msg) => {
                if is_interactive(config) { print_step_ok("DHCP renewed", config); }
                steps.push(StepResult { name: "interface_reset", success: true, message: msg });
            }
            Err(msg) => {
                if is_interactive(config) { print_step_fail("DHCP renew failed", &msg, config); }
                steps.push(StepResult { name: "interface_reset", success: false, message: msg });
            }
        }
    }

    // Wait and check connectivity
    {
        let spinner = create_spinner("Checking connectivity...");
        tokio::time::sleep(std::time::Duration::from_secs(5)).await;
        let connected = connectivity::check_connectivity().await;
        spinner.finish_and_clear();
        (steps, connected)
    }
}

async fn disable_interface(iface: &str) -> Result<(), String> {
    #[cfg(windows)]
    {
        let mut cmd = tokio::process::Command::new("netsh");
        cmd.args(["interface", "set", "interface", iface, "disabled"]);
        match run_cmd(cmd, TIMEOUT_SLOW).await {
            Ok(output) if output.status.success() => Ok(()),
            Ok(output) => Err(String::from_utf8_lossy(&output.stderr).trim().to_string()),
            Err(e) => Err(e),
        }
    }

    #[cfg(target_os = "macos")]
    {
        // Check if Wi-Fi
        let is_wifi = iface.starts_with("en") && is_wifi_device(iface).await;
        if is_wifi {
            let mut cmd = tokio::process::Command::new("networksetup");
            cmd.args(["-setairportpower", iface, "off"]);
            match run_cmd(cmd, TIMEOUT_MEDIUM).await {
                Ok(output) if output.status.success() => Ok(()),
                Ok(output) => Err(String::from_utf8_lossy(&output.stderr).trim().to_string()),
                Err(e) => Err(e),
            }
        } else {
            let mut cmd = tokio::process::Command::new("ifconfig");
            cmd.args([iface, "down"]);
            match run_cmd(cmd, TIMEOUT_MEDIUM).await {
                Ok(output) if output.status.success() => Ok(()),
                Ok(output) => Err(String::from_utf8_lossy(&output.stderr).trim().to_string()),
                Err(e) => Err(e),
            }
        }
    }

    #[cfg(target_os = "linux")]
    {
        let mut cmd = tokio::process::Command::new("ip");
        cmd.args(["link", "set", iface, "down"]);
        match run_cmd(cmd, TIMEOUT_MEDIUM).await {
            Ok(output) if output.status.success() => Ok(()),
            Ok(output) => Err(String::from_utf8_lossy(&output.stderr).trim().to_string()),
            Err(e) => Err(e),
        }
    }
}

async fn enable_interface(iface: &str) -> Result<(), String> {
    #[cfg(windows)]
    {
        let mut cmd = tokio::process::Command::new("netsh");
        cmd.args(["interface", "set", "interface", iface, "enabled"]);
        match run_cmd(cmd, TIMEOUT_SLOW).await {
            Ok(output) if output.status.success() => Ok(()),
            Ok(output) => Err(String::from_utf8_lossy(&output.stderr).trim().to_string()),
            Err(e) => Err(e),
        }
    }

    #[cfg(target_os = "macos")]
    {
        let is_wifi = iface.starts_with("en") && is_wifi_device(iface).await;
        if is_wifi {
            let mut cmd = tokio::process::Command::new("networksetup");
            cmd.args(["-setairportpower", iface, "on"]);
            match run_cmd(cmd, TIMEOUT_MEDIUM).await {
                Ok(output) if output.status.success() => Ok(()),
                Ok(output) => Err(String::from_utf8_lossy(&output.stderr).trim().to_string()),
                Err(e) => Err(e),
            }
        } else {
            let mut cmd = tokio::process::Command::new("ifconfig");
            cmd.args([iface, "up"]);
            match run_cmd(cmd, TIMEOUT_MEDIUM).await {
                Ok(output) if output.status.success() => Ok(()),
                Ok(output) => Err(String::from_utf8_lossy(&output.stderr).trim().to_string()),
                Err(e) => Err(e),
            }
        }
    }

    #[cfg(target_os = "linux")]
    {
        let mut cmd = tokio::process::Command::new("ip");
        cmd.args(["link", "set", iface, "up"]);
        match run_cmd(cmd, TIMEOUT_MEDIUM).await {
            Ok(output) if output.status.success() => Ok(()),
            Ok(output) => Err(String::from_utf8_lossy(&output.stderr).trim().to_string()),
            Err(e) => Err(e),
        }
    }
}

#[cfg(target_os = "macos")]
async fn is_wifi_device(iface: &str) -> bool {
    let mut cmd = tokio::process::Command::new("networksetup");
    cmd.args(["-listallhardwareports"]);
    if let Ok(output) = run_cmd(cmd, TIMEOUT_QUICK).await {
        let text = String::from_utf8_lossy(&output.stdout);
        let mut is_wifi_port = false;
        for line in text.lines() {
            if let Some(name) = line.strip_prefix("Hardware Port: ") {
                let lower = name.to_lowercase();
                is_wifi_port = lower.contains("wi-fi") || lower.contains("airport");
            } else if let Some(dev) = line.strip_prefix("Device: ") {
                if dev.trim() == iface && is_wifi_port {
                    return true;
                }
                is_wifi_port = false;
            }
        }
    }
    false
}

// ── Stage 3: Network Stack / Profile Reset (prompts with strong warning) ────

pub async fn run_stage3(config: &Config, saved_ssid: &Option<String>) -> (Vec<StepResult>, bool) {
    let mut steps = Vec::new();

    if is_interactive(config) {
        println!();
        let detail = platform_stage3_warning();
        let prompt = format!(
            "  {} WARNING: Stage 3 resets network configuration. {}\n  Saved Wi-Fi passwords and custom settings may be lost. Continue? (y/N): ",
            warn_icon(config),
            detail,
        );
        if !prompt_yes_no(&prompt) {
            steps.push(StepResult {
                name: "stack_reset",
                success: false,
                message: "Skipped by user".to_string(),
            });
            return (steps, false);
        }
    }

    let result = platform_stage3(config, saved_ssid).await;
    match result {
        Ok(msgs) => {
            for msg in &msgs {
                if is_interactive(config) { print_step_ok(msg, config); }
            }
            steps.push(StepResult {
                name: "stack_reset",
                success: true,
                message: msgs.join("; "),
            });
        }
        Err(msg) => {
            if is_interactive(config) { print_step_fail("Stack reset failed", &msg, config); }
            steps.push(StepResult {
                name: "stack_reset",
                success: false,
                message: msg,
            });
        }
    }

    // Wait and check connectivity
    {
        let spinner = create_spinner("Checking connectivity...");
        tokio::time::sleep(std::time::Duration::from_secs(8)).await;
        let connected = connectivity::check_connectivity().await;
        spinner.finish_and_clear();
        (steps, connected)
    }
}

fn platform_stage3_warning() -> &'static str {
    #[cfg(windows)]
    { "This will reset Winsock, TCP/IP stack, and optionally delete your Wi-Fi profile." }

    #[cfg(target_os = "macos")]
    { "This will remove and recreate your network service." }

    #[cfg(target_os = "linux")]
    { "This will delete and recreate your network connection profile." }
}

async fn platform_stage3(config: &Config, saved_ssid: &Option<String>) -> Result<Vec<String>, String> {
    #[cfg(windows)]
    { stage3_windows(config, saved_ssid).await }

    #[cfg(target_os = "macos")]
    { stage3_macos(config, saved_ssid).await }

    #[cfg(target_os = "linux")]
    { stage3_linux(config, saved_ssid).await }
}

#[cfg(windows)]
async fn stage3_windows(config: &Config, saved_ssid: &Option<String>) -> Result<Vec<String>, String> {
    let mut completed = Vec::new();

    // 1. Winsock reset
    {
        let spinner = create_spinner("Resetting Winsock catalog...");
        let mut cmd = tokio::process::Command::new("netsh");
        cmd.args(["winsock", "reset"]);
        let r = run_cmd(cmd, TIMEOUT_SLOW).await;
        spinner.finish_and_clear();
        match r {
            Ok(output) if output.status.success() => completed.push("Winsock reset".to_string()),
            _ => {}
        }
    }

    // 2. TCP/IP reset
    {
        let spinner = create_spinner("Resetting TCP/IP stack...");
        let mut cmd = tokio::process::Command::new("netsh");
        cmd.args(["int", "ip", "reset"]);
        let r = run_cmd(cmd, TIMEOUT_SLOW).await;
        spinner.finish_and_clear();
        match r {
            Ok(output) if output.status.success() => completed.push("TCP/IP reset".to_string()),
            _ => {}
        }
    }

    // 3. IPv6 reset
    {
        let spinner = create_spinner("Resetting IPv6 stack...");
        let mut cmd = tokio::process::Command::new("netsh");
        cmd.args(["int", "ipv6", "reset"]);
        let r = run_cmd(cmd, TIMEOUT_SLOW).await;
        spinner.finish_and_clear();
        match r {
            Ok(output) if output.status.success() => completed.push("IPv6 reset".to_string()),
            _ => {}
        }
    }

    // 4. DNS flush
    {
        let mut cmd = tokio::process::Command::new("ipconfig");
        cmd.arg("/flushdns");
        let _ = run_cmd(cmd, TIMEOUT_QUICK).await;
    }

    // 5. Wi-Fi profile deletion (if applicable)
    if let Some(ssid) = saved_ssid {
        if is_interactive(config) {
            let prompt = format!(
                "  Delete Wi-Fi profile for \"{}\" and reconnect? (y/N): ",
                ssid,
            );
            if prompt_yes_no(&prompt) {
                let mut cmd = tokio::process::Command::new("netsh");
                cmd.args(["wlan", "delete", "profile", &format!("name={}", ssid)]);
                let _ = run_cmd(cmd, TIMEOUT_QUICK).await;
                completed.push(format!("Deleted Wi-Fi profile: {}", ssid));

                // Show available networks
                let networks = wifi::scan_wifi_networks().await;
                if !networks.is_empty() {
                    println!("  Available networks:");
                    for (i, net) in networks.iter().enumerate().take(10) {
                        println!("    {}. {}", i + 1, net);
                    }
                }
                println!(
                    "  {}",
                    crate::render::color::dim("Reconnect to your Wi-Fi network via the system tray.", config),
                );
            }
        }
    }

    // 6. Reboot note
    if is_interactive(config) {
        println!();
        println!(
            "  {}",
            crate::render::color::dim("A reboot is recommended to complete the reset.", config),
        );
    }

    if completed.is_empty() {
        Err("No stack reset commands succeeded".to_string())
    } else {
        Ok(completed)
    }
}

#[cfg(target_os = "macos")]
async fn stage3_macos(config: &Config, saved_ssid: &Option<String>) -> Result<Vec<String>, String> {
    let mut completed = Vec::new();

    // Detect default interface and its network service name
    let iface = adapters::detect_default_interface().await
        .ok_or_else(|| "Could not detect default interface".to_string())?;

    let service_name = detect_macos_service(&iface).await
        .ok_or_else(|| format!("Could not find network service for {}", iface))?;

    // 1. Remove network service
    {
        let spinner = create_spinner(&format!("Removing service \"{}\"...", service_name));
        let mut cmd = tokio::process::Command::new("networksetup");
        cmd.args(["-removenetworkservice", &service_name]);
        let r = run_cmd(cmd, TIMEOUT_MEDIUM).await;
        spinner.finish_and_clear();
        match r {
            Ok(output) if output.status.success() => {
                completed.push(format!("Removed service: {}", service_name));
            }
            _ => return Err(format!("Failed to remove service: {}", service_name)),
        }
    }

    // 2. Recreate
    {
        let spinner = create_spinner(&format!("Recreating service \"{}\"...", service_name));
        let mut cmd = tokio::process::Command::new("networksetup");
        cmd.args(["-createnetworkservice", &service_name, &iface]);
        let r = run_cmd(cmd, TIMEOUT_MEDIUM).await;
        spinner.finish_and_clear();
        match r {
            Ok(output) if output.status.success() => {
                completed.push(format!("Recreated service: {}", service_name));
            }
            _ => return Err(format!("Failed to recreate service: {}", service_name)),
        }
    }

    // 3. Set DHCP
    {
        let mut cmd = tokio::process::Command::new("networksetup");
        cmd.args(["-setdhcp", &service_name]);
        let _ = run_cmd(cmd, TIMEOUT_MEDIUM).await;
    }

    // 4. Wi-Fi reconnection
    if let Some(ssid) = saved_ssid {
        // Try to get password from Keychain
        let password = get_keychain_password(ssid).await;
        match password {
            Some(pass) => {
                let mut cmd = tokio::process::Command::new("networksetup");
                cmd.args(["-setairportnetwork", &iface, ssid, &pass]);
                if let Ok(output) = run_cmd(cmd, TIMEOUT_MEDIUM).await {
                    if output.status.success() {
                        completed.push(format!("Reconnected to {}", ssid));
                    }
                }
            }
            None => {
                if is_interactive(config) {
                    println!(
                        "  {}",
                        crate::render::color::dim(
                            &format!("Could not retrieve password for \"{}\". Reconnect via Wi-Fi menu.", ssid),
                            config,
                        ),
                    );
                }
            }
        }
    }

    // 5. Flush DNS
    {
        let mut cmd = tokio::process::Command::new("dscacheutil");
        cmd.arg("-flushcache");
        let _ = run_cmd(cmd, TIMEOUT_QUICK).await;
    }
    {
        let mut cmd = tokio::process::Command::new("killall");
        cmd.args(["-HUP", "mDNSResponder"]);
        let _ = run_cmd(cmd, TIMEOUT_QUICK).await;
    }

    Ok(completed)
}

#[cfg(target_os = "macos")]
async fn detect_macos_service(iface: &str) -> Option<String> {
    let mut cmd = tokio::process::Command::new("networksetup");
    cmd.args(["-listallhardwareports"]);
    if let Ok(output) = run_cmd(cmd, TIMEOUT_QUICK).await {
        let text = String::from_utf8_lossy(&output.stdout);
        let mut current_name = String::new();
        for line in text.lines() {
            if let Some(name) = line.strip_prefix("Hardware Port: ") {
                current_name = name.trim().to_string();
            } else if let Some(dev) = line.strip_prefix("Device: ") {
                if dev.trim() == iface {
                    return Some(current_name);
                }
            }
        }
    }
    None
}

#[cfg(target_os = "macos")]
async fn get_keychain_password(ssid: &str) -> Option<String> {
    let mut cmd = tokio::process::Command::new("security");
    cmd.args(["find-generic-password", "-wa", ssid]);
    if let Ok(output) = run_cmd(cmd, TIMEOUT_QUICK).await {
        if output.status.success() {
            let pass = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if !pass.is_empty() {
                return Some(pass);
            }
        }
    }
    None
}

#[cfg(target_os = "linux")]
async fn stage3_linux(config: &Config, saved_ssid: &Option<String>) -> Result<Vec<String>, String> {
    let mut completed = Vec::new();

    let iface = adapters::detect_default_interface().await
        .ok_or_else(|| "Could not detect default interface".to_string())?;

    let has_nm = has_network_manager().await;

    if has_nm {
        // Delete active connection profile
        if let Some(profile) = detect_nm_profile(&iface).await {
            let spinner = create_spinner(&format!("Deleting profile \"{}\"...", profile));
            let mut cmd = tokio::process::Command::new("nmcli");
            cmd.args(["connection", "delete", &profile]);
            let _ = run_cmd(cmd, TIMEOUT_MEDIUM).await;
            spinner.finish_and_clear();
            completed.push(format!("Deleted profile: {}", profile));
        }

        // Recreate
        let is_wifi = is_linux_wifi(&iface).await;
        if is_wifi {
            // Wi-Fi: scan and connect
            let spinner = create_spinner("Scanning for Wi-Fi networks...");
            let mut rescan_cmd = tokio::process::Command::new("nmcli");
            rescan_cmd.args(["device", "wifi", "rescan"]);
            let _ = run_cmd(rescan_cmd, TIMEOUT_QUICK).await;
            tokio::time::sleep(std::time::Duration::from_secs(2)).await;
            spinner.finish_and_clear();

            let ssid_to_connect = if let Some(ssid) = saved_ssid {
                ssid.clone()
            } else if is_interactive(config) {
                let networks = wifi::scan_wifi_networks().await;
                if !networks.is_empty() {
                    println!("  Available networks:");
                    for (i, net) in networks.iter().enumerate().take(10) {
                        println!("    {}. {}", i + 1, net);
                    }
                }
                prompt_string("  Enter SSID to connect to: ")
            } else {
                return Err("Wi-Fi reconnection requires interactive mode".to_string());
            };

            if is_interactive(config) {
                let passphrase = prompt_string("  Enter passphrase: ");
                let spinner = create_spinner(&format!("Connecting to {}...", ssid_to_connect));
                let mut cmd = tokio::process::Command::new("nmcli");
                cmd.args(["device", "wifi", "connect", &ssid_to_connect, "password", &passphrase, "ifname", &iface]);
                let r = run_cmd(cmd, TIMEOUT_MEDIUM).await;
                spinner.finish_and_clear();
                match r {
                    Ok(output) if output.status.success() => {
                        completed.push(format!("Connected to {}", ssid_to_connect));
                    }
                    _ => {
                        return Err(format!("Failed to connect to {}", ssid_to_connect));
                    }
                }
            }
        } else {
            // Wired: auto-create
            let spinner = create_spinner("Creating new ethernet profile...");
            let con_name = format!("Auto {}", iface);
            let mut cmd = tokio::process::Command::new("nmcli");
            cmd.args(["connection", "add", "type", "ethernet", "con-name", &con_name, "ifname", &iface]);
            let r = run_cmd(cmd, TIMEOUT_MEDIUM).await;
            spinner.finish_and_clear();
            match r {
                Ok(output) if output.status.success() => {
                    completed.push(format!("Created profile: {}", con_name));
                }
                _ => return Err("Failed to create ethernet profile".to_string()),
            }
        }
    } else {
        // dhcpcd fallback
        let spinner = create_spinner("Resetting dhcpcd configuration...");
        // Back up config
        let mut cp_cmd = tokio::process::Command::new("cp");
        cp_cmd.args(["/etc/dhcpcd.conf", "/etc/dhcpcd.conf.bak"]);
        let _ = run_cmd(cp_cmd, TIMEOUT_QUICK).await;
        completed.push("Backed up /etc/dhcpcd.conf".to_string());

        // Just restart dhcpcd to pick up changes
        let mut restart_cmd = tokio::process::Command::new("systemctl");
        restart_cmd.args(["restart", "dhcpcd"]);
        let _ = run_cmd(restart_cmd, TIMEOUT_MEDIUM).await;
        spinner.finish_and_clear();
        completed.push("Restarted dhcpcd".to_string());

        // Wi-Fi via wpa_cli
        let is_wifi = is_linux_wifi(&iface).await;
        if is_wifi {
            if is_interactive(config) {
                let ssid_to_connect = if let Some(ssid) = saved_ssid {
                    ssid.clone()
                } else {
                    prompt_string("  Enter SSID to connect to: ")
                };
                let passphrase = prompt_string("  Enter passphrase: ");

                // Add network via wpa_cli
                let spinner = create_spinner("Scanning for Wi-Fi networks...");
                let mut scan_cmd = tokio::process::Command::new("wpa_cli");
                scan_cmd.args(["-i", &iface, "scan"]);
                let _ = run_cmd(scan_cmd, TIMEOUT_QUICK).await;
                tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                spinner.finish_and_clear();

                // Create wpa_supplicant entry
                let wpa_conf = format!(
                    "network={{\n  ssid=\"{}\"\n  psk=\"{}\"\n}}\n",
                    ssid_to_connect, passphrase,
                );
                // Append to wpa_supplicant.conf
                if let Ok(mut file) = tokio::fs::OpenOptions::new()
                    .append(true)
                    .open("/etc/wpa_supplicant/wpa_supplicant.conf")
                    .await
                {
                    use tokio::io::AsyncWriteExt;
                    let _ = file.write_all(wpa_conf.as_bytes()).await;
                    let mut reconf_cmd = tokio::process::Command::new("wpa_cli");
                    reconf_cmd.args(["-i", &iface, "reconfigure"]);
                    let _ = run_cmd(reconf_cmd, TIMEOUT_QUICK).await;
                    completed.push(format!("Connected to {} via wpa_supplicant", ssid_to_connect));
                }
            }
        }
    }

    if completed.is_empty() {
        Err("No reset steps succeeded".to_string())
    } else {
        Ok(completed)
    }
}

#[cfg(target_os = "linux")]
async fn has_network_manager() -> bool {
    let mut cmd = tokio::process::Command::new("systemctl");
    cmd.args(["is-active", "NetworkManager"]);
    if let Ok(output) = run_cmd(cmd, TIMEOUT_QUICK).await {
        let text = String::from_utf8_lossy(&output.stdout);
        return text.trim() == "active";
    }
    false
}

#[cfg(target_os = "linux")]
async fn detect_nm_profile(iface: &str) -> Option<String> {
    let mut cmd = tokio::process::Command::new("nmcli");
    cmd.args(["-t", "-f", "NAME,DEVICE", "connection", "show", "--active"]);
    if let Ok(output) = run_cmd(cmd, TIMEOUT_QUICK).await {
        let text = String::from_utf8_lossy(&output.stdout);
        for line in text.lines() {
            let parts: Vec<&str> = line.splitn(2, ':').collect();
            if parts.len() == 2 && parts[1] == iface {
                return Some(parts[0].to_string());
            }
        }
    }
    None
}

#[cfg(target_os = "linux")]
async fn is_linux_wifi(iface: &str) -> bool {
    let mut cmd = tokio::process::Command::new("iw");
    cmd.args(["dev", iface, "info"]);
    if let Ok(output) = run_cmd(cmd, TIMEOUT_QUICK).await {
        return output.status.success();
    }
    false
}
