use crate::config::Config;
use crate::render::color;
use crate::render::progress::create_spinner;

use super::adapters;
use super::arp;
#[allow(unused_imports)]
use super::cmd::{run_cmd, TIMEOUT_MEDIUM, TIMEOUT_QUICK, TIMEOUT_SLOW};
use super::connectivity;
use super::dhcp;
use super::dns;
#[cfg(any(windows, target_os = "linux"))]
use super::wifi;
use super::{print_step_fail, print_step_ok, warn_icon, StepResult};
#[cfg(target_os = "linux")]
use crate::actions::prompt_string;
use crate::actions::{flush_dns_platform, is_interactive, prompt_yes_no};

// ── Stage 1: Service Restart (automatic, safe) ─────────────────────────────

pub async fn run_stage1(config: &Config) -> (Vec<StepResult>, bool) {
    let mut steps = Vec::new();

    // Reset DNS to Automatic (DHCP) — clear any custom/broken DNS servers first
    {
        let spinner = create_spinner("Resetting DNS to automatic (DHCP)...");
        match adapters::detect_default_interface().await {
            Some(iface) => {
                let service_name = detect_service_name(&iface).await;
                let result =
                    dns::set_dns_servers(&iface, &service_name, dns::DnsProvider::Automatic).await;
                spinner.finish_and_clear();
                match result {
                    Ok(msg) => {
                        if is_interactive(config) {
                            print_step_ok("DNS reset to automatic (DHCP)", config);
                        }
                        steps.push(StepResult {
                            name: "dns_reset_auto",
                            success: true,
                            message: msg,
                        });
                    }
                    Err(msg) => {
                        if is_interactive(config) {
                            print_step_fail("Failed to reset DNS to automatic", &msg, config);
                        }
                        steps.push(StepResult {
                            name: "dns_reset_auto",
                            success: false,
                            message: msg,
                        });
                    }
                }
            }
            None => {
                spinner.finish_and_clear();
                if is_interactive(config) {
                    print_step_fail("Could not detect interface for DNS reset", "", config);
                }
                steps.push(StepResult {
                    name: "dns_reset_auto",
                    success: false,
                    message: "Could not detect default interface".to_string(),
                });
            }
        }
    }

    // DNS flush
    {
        let spinner = create_spinner("Flushing DNS cache...");
        let result = flush_dns_platform().await;
        spinner.finish_and_clear();
        match result {
            Ok(msg) => {
                if is_interactive(config) {
                    print_step_ok("DNS cache flushed", config);
                }
                steps.push(StepResult {
                    name: "dns_flush",
                    success: true,
                    message: msg,
                });
            }
            Err(msg) => {
                if is_interactive(config) {
                    print_step_fail("Failed to flush DNS cache", &msg, config);
                }
                steps.push(StepResult {
                    name: "dns_flush",
                    success: false,
                    message: msg,
                });
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
                if is_interactive(config) {
                    print_step_ok("ARP cache flushed", config);
                }
                steps.push(StepResult {
                    name: "arp_flush",
                    success: true,
                    message: msg,
                });
            }
            Err(msg) => {
                if is_interactive(config) {
                    print_step_fail("Failed to flush ARP cache", &msg, config);
                }
                steps.push(StepResult {
                    name: "arp_flush",
                    success: false,
                    message: msg,
                });
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
                if is_interactive(config) {
                    print_step_ok("Network services restarted", config);
                }
                steps.push(StepResult {
                    name: "service_restart",
                    success: true,
                    message: msg,
                });
            }
            Err(msg) => {
                if is_interactive(config) {
                    print_step_fail("Failed to restart services", &msg, config);
                }
                steps.push(StepResult {
                    name: "service_restart",
                    success: false,
                    message: msg,
                });
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
                if is_interactive(config) {
                    print_step_ok("DHCP lease renewed", config);
                }
                steps.push(StepResult {
                    name: "dhcp_renew",
                    success: true,
                    message: msg,
                });
            }
            Err(msg) => {
                if is_interactive(config) {
                    print_step_fail("Failed to renew DHCP lease", &msg, config);
                }
                steps.push(StepResult {
                    name: "dhcp_renew",
                    success: false,
                    message: msg,
                });
            }
        }
    }

    // Wait and check connectivity
    let http_ok = {
        let spinner = create_spinner("Waiting for network to reconnect...");
        let connected = connectivity::wait_for_connectivity(&spinner).await;
        spinner.finish_and_clear();
        connected
    };

    if !http_ok {
        return (steps, false);
    }

    // DNS verification after connectivity passes
    let dns_ok = {
        let spinner = create_spinner("Verifying DNS resolution...");
        let ok = connectivity::verify_dns_stability(&spinner).await;
        spinner.finish_and_clear();
        ok
    };

    if dns_ok {
        if is_interactive(config) {
            print_step_ok("DNS resolution verified", config);
        }
        return (steps, true);
    }

    // DNS failed — offer the user a chance to change DNS servers before Stage 2
    if is_interactive(config) {
        print_step_fail("DNS is not resolving correctly", "", config);
    }

    let iface = adapters::detect_default_interface()
        .await
        .unwrap_or_default();
    let dns_fixed = handle_dns_fallback_prompted(config, &iface, &mut steps).await;
    (steps, dns_fixed)
}

/// Restart platform networking services.
pub async fn restart_services() -> Result<String, String> {
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
                cmd.args([
                    "-NoProfile",
                    "-Command",
                    &format!("Restart-Service -Name '{}' -Force", internal),
                ]);
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
            Ok(
                "Services protected (PPL); DNS flush and DHCP renew handle cache refresh"
                    .to_string(),
            )
        } else {
            Ok(format!("Restarted: {}", restarted.join(", ")))
        }
    }

    #[cfg(target_os = "macos")]
    {
        let mut cmd = tokio::process::Command::new("killall");
        cmd.args(["-HUP", "configd"]);
        match run_cmd(cmd, TIMEOUT_QUICK).await {
            Ok(output) if output.status.success() => Ok("Restarted configd".to_string()),
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
        println!("  Resetting interface: {}", color::cyan(&iface, config),);
    }

    // Disable/down
    {
        let spinner = create_spinner(&format!("Disabling {}...", iface));
        let result = disable_interface(&iface).await;
        spinner.finish_and_clear();
        match result {
            Ok(_) => {
                if is_interactive(config) {
                    print_step_ok(&format!("Disabled {}", iface), config);
                }
            }
            Err(msg) => {
                if is_interactive(config) {
                    print_step_fail(&format!("Failed to disable {}", iface), &msg, config);
                }
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
                if is_interactive(config) {
                    print_step_ok(&format!("Re-enabled {}", iface), config);
                }
            }
            Err(_) => {
                // Retry once — leaving interface disabled is worse than a slow retry
                let spinner = create_spinner(&format!("Retrying re-enable {}...", iface));
                tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                let retry = enable_interface(&iface).await;
                spinner.finish_and_clear();
                match retry {
                    Ok(_) => {
                        if is_interactive(config) {
                            print_step_ok(&format!("Re-enabled {} (retry)", iface), config);
                        }
                    }
                    Err(msg) => {
                        if is_interactive(config) {
                            print_step_fail(
                                &format!("Failed to re-enable {}", iface),
                                &msg,
                                config,
                            );
                        }
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

    // Wait for Wi-Fi to associate before DHCP (macOS needs ~5s after re-enable)
    {
        let spinner = create_spinner("Waiting for link to establish...");
        tokio::time::sleep(std::time::Duration::from_secs(5)).await;
        spinner.finish_and_clear();
    }

    // Targeted DHCP renew
    {
        let spinner = create_spinner("Renewing DHCP on interface...");
        let result = adapters::renew_dhcp_on_interface(&iface).await;
        spinner.finish_and_clear();
        match result {
            Ok(msg) => {
                if is_interactive(config) {
                    print_step_ok("DHCP renewed", config);
                }
                steps.push(StepResult {
                    name: "interface_reset",
                    success: true,
                    message: msg,
                });
            }
            Err(msg) => {
                if is_interactive(config) {
                    print_step_fail("DHCP renew failed", &msg, config);
                }
                steps.push(StepResult {
                    name: "interface_reset",
                    success: false,
                    message: msg,
                });
            }
        }
    }

    // Wait and check connectivity
    let http_ok = {
        let spinner = create_spinner("Waiting for network to reconnect...");
        let connected = connectivity::wait_for_connectivity(&spinner).await;
        spinner.finish_and_clear();
        connected
    };

    if !http_ok {
        return (steps, false);
    }

    // DNS verification after connectivity passes
    let dns_ok = {
        let spinner = create_spinner("Verifying DNS resolution...");
        let ok = connectivity::verify_dns_stability(&spinner).await;
        spinner.finish_and_clear();
        ok
    };

    if dns_ok {
        if is_interactive(config) {
            print_step_ok("DNS resolution verified", config);
        }
        return (steps, true);
    }

    // DNS failed — prompt user for DNS server choice
    let dns_fixed = handle_dns_fallback_prompted(config, &iface, &mut steps).await;
    (steps, dns_fixed)
}

pub async fn disable_interface(iface: &str) -> Result<(), String> {
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

pub async fn enable_interface(iface: &str) -> Result<(), String> {
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
                if is_interactive(config) {
                    print_step_ok(msg, config);
                }
            }
            steps.push(StepResult {
                name: "stack_reset",
                success: true,
                message: msgs.join("; "),
            });
        }
        Err(msg) => {
            if is_interactive(config) {
                print_step_fail("Stack reset failed", &msg, config);
            }
            steps.push(StepResult {
                name: "stack_reset",
                success: false,
                message: msg,
            });
        }
    }

    // Wait and check connectivity
    let http_ok = {
        let spinner = create_spinner("Waiting for network to reconnect...");
        let connected = connectivity::wait_for_connectivity(&spinner).await;
        spinner.finish_and_clear();
        connected
    };

    if !http_ok {
        return (steps, false);
    }

    // DNS verification after connectivity passes
    let dns_ok = {
        let spinner = create_spinner("Verifying DNS resolution...");
        let ok = connectivity::verify_dns_stability(&spinner).await;
        spinner.finish_and_clear();
        ok
    };

    if dns_ok {
        if is_interactive(config) {
            print_step_ok("DNS resolution verified", config);
        }
        return (steps, true);
    }

    // DNS failed — auto-apply Google DNS (no prompt in Stage 3)
    let iface = adapters::detect_default_interface()
        .await
        .unwrap_or_default();
    let dns_fixed = handle_dns_fallback_auto(config, &iface, &mut steps).await;
    (steps, dns_fixed)
}

fn platform_stage3_warning() -> &'static str {
    #[cfg(windows)]
    {
        "This will reset Winsock, TCP/IP stack, and optionally delete your Wi-Fi profile."
    }

    #[cfg(target_os = "macos")]
    {
        "This will remove and recreate your network service."
    }

    #[cfg(target_os = "linux")]
    {
        "This will delete and recreate your network connection profile."
    }
}

pub async fn platform_stage3(
    config: &Config,
    saved_ssid: &Option<String>,
) -> Result<Vec<String>, String> {
    #[cfg(windows)]
    {
        stage3_windows(config, saved_ssid).await
    }

    #[cfg(target_os = "macos")]
    {
        stage3_macos(config, saved_ssid).await
    }

    #[cfg(target_os = "linux")]
    {
        stage3_linux(config, saved_ssid).await
    }
}

#[cfg(windows)]
async fn stage3_windows(
    config: &Config,
    saved_ssid: &Option<String>,
) -> Result<Vec<String>, String> {
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
                    crate::render::color::dim(
                        "Reconnect to your Wi-Fi network via the system tray.",
                        config
                    ),
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
#[derive(Debug, Clone, PartialEq, Eq)]
struct CommandSpec {
    program: &'static str,
    args: Vec<String>,
}

#[cfg(target_os = "macos")]
impl CommandSpec {
    fn new(program: &'static str, args: impl IntoIterator<Item = impl Into<String>>) -> Self {
        Self {
            program,
            args: args.into_iter().map(Into::into).collect(),
        }
    }
}

#[cfg(target_os = "macos")]
#[derive(Debug, Clone, PartialEq, Eq)]
enum MacosIpv4Snapshot {
    Dhcp,
    Manual {
        ip: String,
        subnet: String,
        router: String,
    },
    Unknown,
}

#[cfg(target_os = "macos")]
#[derive(Debug, Clone, PartialEq, Eq)]
enum MacosIpv6Snapshot {
    Automatic,
    LinkLocal,
    Off,
    Manual {
        address: String,
        prefix: String,
        router: String,
    },
    Unknown,
}

#[cfg(target_os = "macos")]
#[derive(Debug, Clone, PartialEq, Eq)]
struct MacosProxySnapshot {
    kind: &'static str,
    enabled: bool,
    server: Option<String>,
    port: Option<String>,
}

#[cfg(target_os = "macos")]
impl MacosProxySnapshot {
    fn disabled(kind: &'static str) -> Self {
        Self {
            kind,
            enabled: false,
            server: None,
            port: None,
        }
    }

    fn restore_specs(&self, service: &str) -> Vec<CommandSpec> {
        let set_cmd = match self.kind {
            "secureweb" => "-setsecurewebproxy",
            _ => "-setwebproxy",
        };
        let state_cmd = match self.kind {
            "secureweb" => "-setsecurewebproxystate",
            _ => "-setwebproxystate",
        };

        if self.enabled {
            let server = self.server.clone().unwrap_or_default();
            let port = self.port.clone().unwrap_or_else(|| "0".to_string());
            vec![
                CommandSpec::new("networksetup", [set_cmd, service, &server, &port]),
                CommandSpec::new("networksetup", [state_cmd, service, "on"]),
            ]
        } else {
            vec![CommandSpec::new(
                "networksetup",
                [state_cmd, service, "off"],
            )]
        }
    }
}

#[cfg(target_os = "macos")]
#[derive(Debug, Clone, PartialEq, Eq)]
struct MacosNetworkSnapshot {
    service_name: String,
    dns_servers: Vec<String>,
    service_order: Vec<String>,
    web_proxy: MacosProxySnapshot,
    secure_web_proxy: MacosProxySnapshot,
    ipv4: MacosIpv4Snapshot,
    ipv6: MacosIpv6Snapshot,
}

#[cfg(target_os = "macos")]
impl MacosNetworkSnapshot {
    fn restore_command_specs(&self) -> Vec<CommandSpec> {
        let mut specs = Vec::new();

        let mut dns_args = vec!["-setdnsservers".to_string(), self.service_name.clone()];
        if self.dns_servers.is_empty() {
            dns_args.push("empty".to_string());
        } else {
            dns_args.extend(self.dns_servers.clone());
        }
        specs.push(CommandSpec::new("networksetup", dns_args));

        match &self.ipv4 {
            MacosIpv4Snapshot::Dhcp => {
                specs.push(CommandSpec::new(
                    "networksetup",
                    ["-setdhcp", self.service_name.as_str()],
                ));
            }
            MacosIpv4Snapshot::Manual { ip, subnet, router } => {
                specs.push(CommandSpec::new(
                    "networksetup",
                    [
                        "-setmanual",
                        self.service_name.as_str(),
                        ip.as_str(),
                        subnet.as_str(),
                        router.as_str(),
                    ],
                ));
            }
            MacosIpv4Snapshot::Unknown => {}
        }

        match &self.ipv6 {
            MacosIpv6Snapshot::Automatic => {
                specs.push(CommandSpec::new(
                    "networksetup",
                    ["-setv6automatic", self.service_name.as_str()],
                ));
            }
            MacosIpv6Snapshot::LinkLocal => {
                specs.push(CommandSpec::new(
                    "networksetup",
                    ["-setv6linklocal", self.service_name.as_str()],
                ));
            }
            MacosIpv6Snapshot::Off => {
                specs.push(CommandSpec::new(
                    "networksetup",
                    ["-setv6off", self.service_name.as_str()],
                ));
            }
            MacosIpv6Snapshot::Manual {
                address,
                prefix,
                router,
            } => {
                specs.push(CommandSpec::new(
                    "networksetup",
                    [
                        "-setv6manual",
                        self.service_name.as_str(),
                        address.as_str(),
                        prefix.as_str(),
                        router.as_str(),
                    ],
                ));
            }
            MacosIpv6Snapshot::Unknown => {}
        }

        specs.extend(self.web_proxy.restore_specs(&self.service_name));
        specs.extend(self.secure_web_proxy.restore_specs(&self.service_name));

        if !self.service_order.is_empty() {
            let mut order_args = vec!["-ordernetworkservices".to_string()];
            order_args.extend(self.service_order.clone());
            specs.push(CommandSpec::new("networksetup", order_args));
        }

        specs
    }
}

#[cfg(target_os = "macos")]
async fn run_networksetup(args: &[&str]) -> Option<String> {
    let mut cmd = tokio::process::Command::new("networksetup");
    cmd.args(args);
    let output = run_cmd(cmd, TIMEOUT_MEDIUM).await.ok()?;
    if !output.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&output.stdout).into_owned())
}

#[cfg(target_os = "macos")]
async fn capture_macos_network_snapshot(service_name: &str) -> MacosNetworkSnapshot {
    let ip_info = run_networksetup(&["-getinfo", service_name]).await;
    let dns_servers = run_networksetup(&["-getdnsservers", service_name])
        .await
        .map(|text| parse_macos_dns_servers(&text))
        .unwrap_or_default();
    let service_order = run_networksetup(&["-listnetworkserviceorder"])
        .await
        .map(|text| parse_macos_service_order(&text))
        .unwrap_or_default();
    let web_proxy = run_networksetup(&["-getwebproxy", service_name])
        .await
        .map(|text| parse_macos_proxy("web", &text))
        .unwrap_or_else(|| MacosProxySnapshot::disabled("web"));
    let secure_web_proxy = run_networksetup(&["-getsecurewebproxy", service_name])
        .await
        .map(|text| parse_macos_proxy("secureweb", &text))
        .unwrap_or_else(|| MacosProxySnapshot::disabled("secureweb"));
    let ipv4 = ip_info
        .as_deref()
        .map(parse_macos_ipv4)
        .unwrap_or(MacosIpv4Snapshot::Unknown);
    let ipv6 = ip_info
        .as_deref()
        .map(parse_macos_ipv6)
        .unwrap_or(MacosIpv6Snapshot::Unknown);

    MacosNetworkSnapshot {
        service_name: service_name.to_string(),
        dns_servers,
        service_order,
        web_proxy,
        secure_web_proxy,
        ipv4,
        ipv6,
    }
}

#[cfg(target_os = "macos")]
fn parse_macos_dns_servers(text: &str) -> Vec<String> {
    if text.contains("There aren't any DNS Servers") {
        return Vec::new();
    }
    text.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(ToString::to_string)
        .collect()
}

#[cfg(target_os = "macos")]
fn parse_macos_service_order(text: &str) -> Vec<String> {
    text.lines()
        .filter_map(|line| {
            let trimmed = line.trim();
            if !trimmed.starts_with('(') {
                return None;
            }
            let (_, name) = trimmed.split_once(") ")?;
            Some(name.trim().trim_start_matches("*").trim().to_string())
        })
        .collect()
}

#[cfg(target_os = "macos")]
fn parse_macos_proxy(kind: &'static str, text: &str) -> MacosProxySnapshot {
    let mut enabled = false;
    let mut server = None;
    let mut port = None;

    for line in text.lines() {
        let trimmed = line.trim();
        if let Some(value) = trimmed.strip_prefix("Enabled:") {
            enabled = value.trim() == "Yes";
        } else if let Some(value) = trimmed.strip_prefix("Server:") {
            let value = value.trim();
            if !value.is_empty() {
                server = Some(value.to_string());
            }
        } else if let Some(value) = trimmed.strip_prefix("Port:") {
            let value = value.trim();
            if !value.is_empty() && value != "0" {
                port = Some(value.to_string());
            }
        }
    }

    MacosProxySnapshot {
        kind,
        enabled,
        server,
        port,
    }
}

#[cfg(target_os = "macos")]
fn parse_macos_ipv4(text: &str) -> MacosIpv4Snapshot {
    if text.contains("DHCP Configuration") {
        return MacosIpv4Snapshot::Dhcp;
    }

    if text.contains("Manual Configuration") {
        let mut ip = String::new();
        let mut subnet = String::new();
        let mut router = String::new();
        for line in text.lines() {
            let trimmed = line.trim();
            if let Some(value) = trimmed.strip_prefix("IP address:") {
                ip = value.trim().to_string();
            } else if let Some(value) = trimmed.strip_prefix("Subnet mask:") {
                subnet = value.trim().to_string();
            } else if let Some(value) = trimmed.strip_prefix("Router:") {
                router = value.trim().to_string();
            }
        }
        if !ip.is_empty() && !subnet.is_empty() && !router.is_empty() {
            return MacosIpv4Snapshot::Manual { ip, subnet, router };
        }
    }

    MacosIpv4Snapshot::Unknown
}

#[cfg(target_os = "macos")]
fn parse_macos_ipv6(text: &str) -> MacosIpv6Snapshot {
    let mut mode = None;
    let mut address = String::new();
    let mut prefix = String::new();
    let mut router = String::new();

    for line in text.lines() {
        let trimmed = line.trim();
        if let Some(value) = trimmed.strip_prefix("IPv6:") {
            let value = value.trim().to_lowercase();
            mode = Some(value);
        } else if let Some(value) = trimmed.strip_prefix("IPv6 IP address:") {
            address = value.trim().to_string();
        } else if let Some(value) = trimmed.strip_prefix("IPv6 Prefix Length:") {
            prefix = value.trim().to_string();
        } else if let Some(value) = trimmed.strip_prefix("IPv6 Router:") {
            router = value.trim().to_string();
        }
    }

    match mode.as_deref() {
        Some(value) if value.contains("automatic") => MacosIpv6Snapshot::Automatic,
        Some(value) if value.contains("link-local") => MacosIpv6Snapshot::LinkLocal,
        Some(value) if value.contains("off") => MacosIpv6Snapshot::Off,
        Some(value) if value.contains("manual") => {
            if !address.is_empty() && !prefix.is_empty() && !router.is_empty() {
                MacosIpv6Snapshot::Manual {
                    address,
                    prefix,
                    router,
                }
            } else {
                MacosIpv6Snapshot::Unknown
            }
        }
        _ => MacosIpv6Snapshot::Unknown,
    }
}

#[cfg(target_os = "macos")]
async fn restore_macos_network_snapshot(snapshot: &MacosNetworkSnapshot) -> Vec<String> {
    let mut restored = Vec::new();

    for spec in snapshot.restore_command_specs() {
        let mut cmd = tokio::process::Command::new(spec.program);
        cmd.args(&spec.args);
        if let Ok(output) = run_cmd(cmd, TIMEOUT_MEDIUM).await {
            if output.status.success() {
                restored.push(format!("{} {}", spec.program, spec.args.join(" ")));
            }
        }
    }

    restored
}

#[cfg(target_os = "macos")]
async fn stage3_macos(config: &Config, saved_ssid: &Option<String>) -> Result<Vec<String>, String> {
    let mut completed = Vec::new();

    // Detect default interface and its network service name
    let iface = adapters::detect_default_interface()
        .await
        .ok_or_else(|| "Could not detect default interface".to_string())?;

    let service_name = detect_macos_service(&iface)
        .await
        .ok_or_else(|| format!("Could not find network service for {}", iface))?;
    let snapshot = capture_macos_network_snapshot(&service_name).await;

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

    // 3. Restore the service's previous settings. This avoids converting a
    // DHCP/default-DNS service into a hard-coded public-DNS service.
    {
        let restored = restore_macos_network_snapshot(&snapshot).await;
        if !restored.is_empty() {
            completed.push("Restored previous macOS network service settings".to_string());
        }
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
                            &format!(
                                "Could not retrieve password for \"{}\". Reconnect via Wi-Fi menu.",
                                ssid
                            ),
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
pub async fn detect_macos_service(iface: &str) -> Option<String> {
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

#[cfg(all(test, target_os = "macos"))]
mod macos_snapshot_tests {
    use super::*;

    #[test]
    fn automatic_dns_restores_empty_dns_marker() {
        let snapshot = MacosNetworkSnapshot {
            service_name: "Wi-Fi".to_string(),
            dns_servers: Vec::new(),
            service_order: vec!["Wi-Fi".to_string(), "Thunderbolt Bridge".to_string()],
            web_proxy: MacosProxySnapshot::disabled("web"),
            secure_web_proxy: MacosProxySnapshot::disabled("secureweb"),
            ipv4: MacosIpv4Snapshot::Dhcp,
            ipv6: MacosIpv6Snapshot::Automatic,
        };

        let specs = snapshot.restore_command_specs();
        assert!(specs.iter().any(|spec| {
            spec.program == "networksetup" && spec.args == vec!["-setdnsservers", "Wi-Fi", "empty"]
        }));
        assert!(specs.iter().any(|spec| {
            spec.program == "networksetup"
                && spec.args == vec!["-ordernetworkservices", "Wi-Fi", "Thunderbolt Bridge"]
        }));
        assert!(specs.iter().any(|spec| {
            spec.program == "networksetup" && spec.args == vec!["-setdhcp", "Wi-Fi"]
        }));
        assert!(specs.iter().any(|spec| {
            spec.program == "networksetup" && spec.args == vec!["-setv6automatic", "Wi-Fi"]
        }));
    }

    #[test]
    fn manual_dns_and_proxy_restore_commands_preserve_values() {
        let snapshot = MacosNetworkSnapshot {
            service_name: "Wi-Fi".to_string(),
            dns_servers: vec!["10.0.0.2".to_string(), "10.0.0.3".to_string()],
            service_order: Vec::new(),
            web_proxy: MacosProxySnapshot {
                kind: "web",
                enabled: true,
                server: Some("proxy.example.test".to_string()),
                port: Some("8080".to_string()),
            },
            secure_web_proxy: MacosProxySnapshot::disabled("secureweb"),
            ipv4: MacosIpv4Snapshot::Manual {
                ip: "10.0.0.42".to_string(),
                subnet: "255.255.255.0".to_string(),
                router: "10.0.0.1".to_string(),
            },
            ipv6: MacosIpv6Snapshot::Off,
        };

        let specs = snapshot.restore_command_specs();
        assert!(specs.iter().any(|spec| {
            spec.program == "networksetup"
                && spec.args == vec!["-setdnsservers", "Wi-Fi", "10.0.0.2", "10.0.0.3"]
        }));
        assert!(specs.iter().any(|spec| {
            spec.program == "networksetup"
                && spec.args == vec!["-setwebproxy", "Wi-Fi", "proxy.example.test", "8080"]
        }));
        assert!(specs.iter().any(|spec| {
            spec.program == "networksetup"
                && spec.args
                    == vec![
                        "-setmanual",
                        "Wi-Fi",
                        "10.0.0.42",
                        "255.255.255.0",
                        "10.0.0.1",
                    ]
        }));
        assert!(specs.iter().any(|spec| {
            spec.program == "networksetup" && spec.args == vec!["-setv6off", "Wi-Fi"]
        }));
    }

    #[test]
    fn service_order_strips_disabled_marker() {
        let parsed = parse_macos_service_order(
            "
(1) Wi-Fi
(2) *Thunderbolt Bridge
",
        );

        assert_eq!(parsed, vec!["Wi-Fi", "Thunderbolt Bridge"]);
    }

    #[test]
    fn ipv6_manual_parse_restores_manual_mode() {
        let parsed = parse_macos_ipv6(
            "
IPv6: Manual
IPv6 IP address: 2001:db8::20
IPv6 Prefix Length: 64
IPv6 Router: 2001:db8::1
",
        );

        assert_eq!(
            parsed,
            MacosIpv6Snapshot::Manual {
                address: "2001:db8::20".to_string(),
                prefix: "64".to_string(),
                router: "2001:db8::1".to_string(),
            }
        );
    }
}

#[cfg(target_os = "linux")]
async fn stage3_linux(config: &Config, saved_ssid: &Option<String>) -> Result<Vec<String>, String> {
    let mut completed = Vec::new();

    let iface = adapters::detect_default_interface()
        .await
        .ok_or_else(|| "Could not detect default interface".to_string())?;

    let has_nm = has_network_manager().await;

    if has_nm {
        let is_wifi = is_linux_wifi(&iface).await;
        let wifi_credentials = if is_wifi {
            if !is_interactive(config) {
                return Err("Wi-Fi profile reset requires interactive mode".to_string());
            }

            let ssid_to_connect = if let Some(ssid) = saved_ssid {
                ssid.clone()
            } else {
                let networks = wifi::scan_wifi_networks().await;
                if !networks.is_empty() {
                    println!("  Available networks:");
                    for (i, net) in networks.iter().enumerate().take(10) {
                        println!("    {}. {}", i + 1, net);
                    }
                }
                prompt_string("  Enter SSID to connect to: ")
            };
            let passphrase = prompt_string("  Enter passphrase (leave blank for open network): ");
            Some((ssid_to_connect, passphrase))
        } else {
            None
        };

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
        if is_wifi {
            // Wi-Fi: scan and connect
            let spinner = create_spinner("Scanning for Wi-Fi networks...");
            let mut rescan_cmd = tokio::process::Command::new("nmcli");
            rescan_cmd.args(["device", "wifi", "rescan"]);
            let _ = run_cmd(rescan_cmd, TIMEOUT_QUICK).await;
            tokio::time::sleep(std::time::Duration::from_secs(2)).await;
            spinner.finish_and_clear();

            if let Some((ssid_to_connect, passphrase)) = wifi_credentials {
                let spinner = create_spinner(&format!("Connecting to {}...", ssid_to_connect));
                let mut cmd = tokio::process::Command::new("nmcli");
                cmd.args(["device", "wifi", "connect", &ssid_to_connect]);
                if !passphrase.is_empty() {
                    cmd.args(["password", &passphrase]);
                }
                cmd.args(["ifname", &iface]);
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
            cmd.args([
                "connection",
                "add",
                "type",
                "ethernet",
                "con-name",
                &con_name,
                "ifname",
                &iface,
            ]);
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
        if is_wifi && is_interactive(config) {
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
                completed.push(format!(
                    "Connected to {} via wpa_supplicant",
                    ssid_to_connect
                ));
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
    cmd.args(["-g", "GENERAL.CONNECTION", "device", "show", iface]);
    if let Ok(output) = run_cmd(cmd, TIMEOUT_QUICK).await {
        if output.status.success() {
            let name = String::from_utf8_lossy(&output.stdout)
                .lines()
                .next()
                .unwrap_or("")
                .trim()
                .to_string();
            if !name.is_empty() && name != "--" {
                return Some(name);
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

// ── DNS fallback helpers ─────────────────────────────────────────────────────

/// Detect the macOS network service name for an interface, or return the
/// interface name itself on other platforms (used as the service_name param).
pub async fn detect_service_name(iface: &str) -> String {
    #[cfg(target_os = "macos")]
    {
        detect_macos_service(iface)
            .await
            .unwrap_or_else(|| iface.to_string())
    }
    #[cfg(not(target_os = "macos"))]
    {
        iface.to_string()
    }
}

/// Stage 2 DNS fallback: prompt user for DNS server choice, test reachability,
/// set DNS, flush cache, wait 5s, re-verify.
async fn handle_dns_fallback_prompted(
    config: &Config,
    iface: &str,
    steps: &mut Vec<StepResult>,
) -> bool {
    let service_name = detect_service_name(iface).await;

    // Prompt user for DNS choice
    let mut provider = dns::prompt_dns_choice(config);

    // Test reachability and adjust if needed
    if provider != dns::DnsProvider::Automatic {
        let spinner = create_spinner("Testing DNS server reachability...");
        let (cf_ok, google_ok) = dns::test_dns_reachability().await;
        spinner.finish_and_clear();
        provider = dns::adjust_for_reachability(provider, cf_ok, google_ok, config);
    }

    // Set DNS servers
    let spinner = create_spinner(&format!("Setting DNS to {}...", provider.label()));
    let result = dns::set_dns_servers(iface, &service_name, provider).await;
    spinner.finish_and_clear();

    match &result {
        Ok(msg) => {
            if is_interactive(config) {
                print_step_ok(msg, config);
            }
            steps.push(StepResult {
                name: "dns_set",
                success: true,
                message: msg.clone(),
            });
        }
        Err(msg) => {
            if is_interactive(config) {
                print_step_fail("Failed to set DNS servers", msg, config);
            }
            steps.push(StepResult {
                name: "dns_set",
                success: false,
                message: msg.clone(),
            });
            return false;
        }
    }

    // Flush DNS cache after changing servers
    let _ = flush_dns_platform().await;

    // Wait for DNS to propagate
    let spinner = create_spinner("Waiting for DNS to propagate...");
    tokio::time::sleep(std::time::Duration::from_secs(5)).await;
    spinner.finish_and_clear();

    // Re-verify DNS
    let spinner = create_spinner("Verifying DNS resolution...");
    let dns_ok = dns::verify_dns().await;
    spinner.finish_and_clear();

    if dns_ok {
        if is_interactive(config) {
            print_step_ok("DNS resolution verified", config);
        }
    } else if is_interactive(config) {
        print_step_fail(
            "DNS still not resolving",
            "May need manual configuration",
            config,
        );
    }

    dns_ok
}

/// Stage 3 DNS fallback: auto-apply Google DNS (no prompt), test reachability,
/// set DNS, flush cache, wait 5s, re-verify.
async fn handle_dns_fallback_auto(
    config: &Config,
    iface: &str,
    steps: &mut Vec<StepResult>,
) -> bool {
    let service_name = detect_service_name(iface).await;

    if is_interactive(config) {
        println!(
            "    {}",
            color::dim(
                "DNS not resolving — auto-applying Google DNS (8.8.8.8 + 8.8.4.4)",
                config
            ),
        );
    }

    // Test reachability to pick best provider
    let spinner = create_spinner("Testing DNS server reachability...");
    let (cf_ok, google_ok) = dns::test_dns_reachability().await;
    spinner.finish_and_clear();

    let provider = dns::adjust_for_reachability(dns::DnsProvider::Google, cf_ok, google_ok, config);

    // Set DNS servers
    let spinner = create_spinner(&format!("Setting DNS to {}...", provider.label()));
    let result = dns::set_dns_servers(iface, &service_name, provider).await;
    spinner.finish_and_clear();

    match &result {
        Ok(msg) => {
            if is_interactive(config) {
                print_step_ok(msg, config);
            }
            steps.push(StepResult {
                name: "dns_set",
                success: true,
                message: msg.clone(),
            });
        }
        Err(msg) => {
            if is_interactive(config) {
                print_step_fail("Failed to set DNS servers", msg, config);
            }
            steps.push(StepResult {
                name: "dns_set",
                success: false,
                message: msg.clone(),
            });
            return false;
        }
    }

    // Flush DNS cache after changing servers
    let _ = flush_dns_platform().await;

    // Wait for DNS to propagate
    let spinner = create_spinner("Waiting for DNS to propagate...");
    tokio::time::sleep(std::time::Duration::from_secs(5)).await;
    spinner.finish_and_clear();

    // Re-verify DNS
    let spinner = create_spinner("Verifying DNS resolution...");
    let dns_ok = dns::verify_dns().await;
    spinner.finish_and_clear();

    if dns_ok {
        if is_interactive(config) {
            print_step_ok("DNS resolution verified", config);
        }
    } else if is_interactive(config) {
        print_step_fail(
            "DNS still not resolving",
            "May need manual configuration",
            config,
        );
    }

    dns_ok
}
