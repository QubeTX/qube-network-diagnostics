use crate::config::Config;
use crate::render::progress::create_spinner;

#[cfg(target_os = "macos")]
use super::cmd::run_macos_mutation;
#[allow(unused_imports)]
use super::cmd::{run_cmd, TIMEOUT_MEDIUM, TIMEOUT_QUICK, TIMEOUT_SLOW};
#[cfg(windows)]
use super::wifi;
// The macOS/Linux stage-3 paths look up the default interface; Windows takes
// it from the caller, so the import is platform-gated to stay -D warnings
// clean on all three OSes.
#[cfg(any(target_os = "macos", target_os = "linux"))]
use super::adapters;
#[cfg(any(windows, target_os = "linux"))]
use crate::actions::is_interactive;
#[cfg(target_os = "linux")]
use crate::actions::prompt_string;
// After the v3.4.0 legacy-orchestrator removal, only the Windows stage-3
// Wi-Fi-profile prompt still asks a yes/no question in this file.
#[cfg(windows)]
use crate::actions::prompt_yes_no;

/// Restart platform networking services.
pub async fn restart_services() -> Result<String, String> {
    #[cfg(windows)]
    {
        let mut restarted = Vec::new();

        // Try sc stop/start with internal service names (works better than net stop/start)
        for (internal, display) in &[("dnscache", "DNS Client"), ("Dhcp", "DHCP Client")] {
            let mut stop_cmd = tokio::process::Command::new("sc");
            stop_cmd.args(["stop", internal]);
            let mut start_cmd = tokio::process::Command::new("sc");
            start_cmd.args(["start", internal]);
            match super::cmd::run_owned_mutation_pair(
                stop_cmd,
                TIMEOUT_MEDIUM,
                std::time::Duration::ZERO,
                start_cmd,
                TIMEOUT_MEDIUM,
            )
            .await
            {
                Ok(pair)
                    if pair
                        .inverse
                        .as_ref()
                        .is_ok_and(|output| output.status.success()) =>
                {
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
                match super::cmd::run_owned_mutation(cmd, TIMEOUT_MEDIUM).await {
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
        if let Ok(output) = super::cmd::run_owned_mutation(nm_cmd, TIMEOUT_MEDIUM).await {
            if output.status.success() {
                return Ok("Restarted NetworkManager".to_string());
            }
        }

        // Fallback to dhcpcd
        let mut dhcpcd_cmd = tokio::process::Command::new("systemctl");
        dhcpcd_cmd.args(["restart", "dhcpcd"]);
        if let Ok(output) = super::cmd::run_owned_mutation(dhcpcd_cmd, TIMEOUT_MEDIUM).await {
            if output.status.success() {
                return Ok("Restarted dhcpcd".to_string());
            }
        }

        Err("Could not restart network services".to_string())
    }
}

pub async fn disable_interface(iface: &str) -> Result<(), String> {
    #[cfg(windows)]
    {
        let mut cmd = tokio::process::Command::new("netsh");
        cmd.args(["interface", "set", "interface", iface, "disabled"]);
        match super::cmd::run_owned_mutation(cmd, TIMEOUT_SLOW).await {
            Ok(output) if output.status.success() => Ok(()),
            Ok(output) => Err(String::from_utf8_lossy(&output.stderr).trim().to_string()),
            Err(e) => Err(e),
        }
    }

    #[cfg(target_os = "macos")]
    {
        // Fail closed if the exact physical service type cannot be mapped;
        // never guess that an unknown Wi-Fi device is wired and run ifconfig.
        let is_wifi = is_wifi_device(iface).await?;
        if is_wifi {
            let mut cmd = tokio::process::Command::new("networksetup");
            cmd.args(["-setairportpower", iface, "off"]);
            match run_macos_mutation(cmd, TIMEOUT_MEDIUM).await {
                Ok(output) if output.status.success() => Ok(()),
                Ok(output) => Err(String::from_utf8_lossy(&output.stderr).trim().to_string()),
                Err(e) => Err(e),
            }
        } else {
            let mut cmd = tokio::process::Command::new("ifconfig");
            cmd.args([iface, "down"]);
            match run_macos_mutation(cmd, TIMEOUT_MEDIUM).await {
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
        match super::cmd::run_owned_mutation(cmd, TIMEOUT_MEDIUM).await {
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
        match super::cmd::run_owned_mutation(cmd, TIMEOUT_SLOW).await {
            Ok(output) if output.status.success() => Ok(()),
            Ok(output) => Err(String::from_utf8_lossy(&output.stderr).trim().to_string()),
            Err(e) => Err(e),
        }
    }

    #[cfg(target_os = "macos")]
    {
        let is_wifi = is_wifi_device(iface).await?;
        if is_wifi {
            let mut cmd = tokio::process::Command::new("networksetup");
            cmd.args(["-setairportpower", iface, "on"]);
            match run_macos_mutation(cmd, TIMEOUT_MEDIUM).await {
                Ok(output) if output.status.success() => Ok(()),
                Ok(output) => Err(String::from_utf8_lossy(&output.stderr).trim().to_string()),
                Err(e) => Err(e),
            }
        } else {
            let mut cmd = tokio::process::Command::new("ifconfig");
            cmd.args([iface, "up"]);
            match run_macos_mutation(cmd, TIMEOUT_MEDIUM).await {
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
        match super::cmd::run_owned_mutation(cmd, TIMEOUT_MEDIUM).await {
            Ok(output) if output.status.success() => Ok(()),
            Ok(output) => Err(String::from_utf8_lossy(&output.stderr).trim().to_string()),
            Err(e) => Err(e),
        }
    }
}

/// Re-enable an interface and verify recovery before a restore token may be
/// resolved. On macOS, an exit-0 toggle is insufficient: prove the mapped
/// physical service, power/link state, expected default route, and reachability.
pub(crate) async fn restore_interface(iface: &str) -> Result<(), String> {
    enable_interface(iface).await?;

    #[cfg(target_os = "macos")]
    {
        let service = macos_service_for_device(iface).await?;
        if !macos_service_enabled(&service.name).await? {
            return Err(format!(
                "macOS network service \"{}\" remains disabled",
                service.name
            ));
        }

        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(10);
        let mut operational = false;
        while tokio::time::Instant::now() < deadline {
            if macos_interface_operational(iface).await? {
                operational = true;
                break;
            }
            tokio::time::sleep(std::time::Duration::from_secs(1)).await;
        }
        if !operational {
            return Err(format!(
                "macOS interface {} did not return to an operational state",
                iface
            ));
        }

        wait_for_macos_route_and_reachability(iface).await?;
    }

    Ok(())
}

#[cfg(target_os = "macos")]
async fn macos_interface_operational(iface: &str) -> Result<bool, String> {
    if is_wifi_device(iface).await? {
        let text = run_networksetup_checked(&["-getairportpower", iface]).await?;
        return parse_macos_airport_power(&text).ok_or_else(|| {
            format!(
                "networksetup returned an unknown Wi-Fi power state for {}",
                iface
            )
        });
    }

    let mut command = tokio::process::Command::new("ifconfig");
    command.arg(iface);
    let output = run_cmd(command, TIMEOUT_QUICK).await?;
    if !output.status.success() {
        return Err(format!("ifconfig could not inspect {}", iface));
    }
    Ok(parse_macos_ifconfig_operational(&String::from_utf8_lossy(
        &output.stdout,
    )))
}

#[cfg(target_os = "macos")]
fn parse_macos_airport_power(text: &str) -> Option<bool> {
    let state = text.trim().rsplit_once(':')?.1.trim();
    match state {
        "On" => Some(true),
        "Off" => Some(false),
        _ => None,
    }
}

#[cfg(target_os = "macos")]
fn parse_macos_ifconfig_operational(text: &str) -> bool {
    let flags_up = text
        .lines()
        .next()
        .and_then(|line| line.split_once("flags="))
        .and_then(|(_, rest)| rest.split_once('<'))
        .and_then(|(_, flags)| flags.split_once('>'))
        .map(|(flags, _)| flags.split(',').any(|flag| flag == "UP"))
        .unwrap_or(false);
    flags_up && (text.contains("status: active") || text.contains("\n\tinet "))
}

#[cfg(target_os = "macos")]
async fn is_wifi_device(iface: &str) -> Result<bool, String> {
    let service = macos_service_for_device(iface).await?;
    classify_macos_hardware_port(&service.hardware_port).ok_or_else(|| {
        format!(
            "Refused to cycle {} because hardware port {:?} could not be classified safely as Wi-Fi or Ethernet",
            iface, service.hardware_port
        )
    })
}

#[cfg(target_os = "macos")]
fn classify_macos_hardware_port(hardware_port: &str) -> Option<bool> {
    let hardware_port = hardware_port.to_ascii_lowercase();
    if hardware_port.contains("wi-fi") || hardware_port.contains("airport") {
        Some(true)
    } else if hardware_port.contains("ethernet") || hardware_port.contains("lan") {
        Some(false)
    } else {
        None
    }
}

pub async fn platform_stage3(
    config: &Config,
    saved_ssid: &Option<String>,
    restore: &super::session::RestoreRegistry,
) -> Result<Vec<String>, String> {
    #[cfg(windows)]
    {
        // Windows stack reset is in-place and self-recovering (no service is
        // removed), so there is nothing to register for restore.
        let _ = restore;
        stage3_windows(config, saved_ssid).await
    }

    #[cfg(target_os = "macos")]
    {
        stage3_macos(config, saved_ssid, restore).await
    }

    #[cfg(target_os = "linux")]
    {
        // Linux deletes/recreates the connection profile in one in-function
        // sequence; the M3 fix makes the delete failure abort before recreate,
        // so there is no half-applied state to register externally.
        let _ = restore;
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
const MACOS_SERVICE_CYCLE_VALIDATED: bool = false;

#[cfg(target_os = "macos")]
#[derive(Debug, Clone, PartialEq, Eq)]
struct MacosNetworkService {
    name: String,
    hardware_port: String,
    device: String,
    enabled: bool,
}

/// Parse the paired service/device records emitted by
/// `networksetup -listnetworkserviceorder`. The display name is the actual
/// user-renamable network service; the hardware-port label is not used as a
/// substitute.
#[cfg(target_os = "macos")]
fn parse_macos_network_service_order(text: &str) -> Vec<MacosNetworkService> {
    let mut services = Vec::new();
    let mut pending: Option<(String, bool)> = None;

    for line in text.lines().map(str::trim).filter(|line| !line.is_empty()) {
        if line.starts_with('(') && !line.starts_with("(Hardware Port:") {
            let Some((marker, raw_name)) = line.split_once(") ") else {
                pending = None;
                continue;
            };
            let marker_disabled = marker == "(*";
            let raw_name = raw_name.trim();
            let name_disabled = raw_name.starts_with('*');
            let name = raw_name.trim_start_matches('*').trim();
            if name.is_empty() {
                pending = None;
            } else {
                pending = Some((name.to_string(), !(marker_disabled || name_disabled)));
            }
            continue;
        }

        let Some(details) = line
            .strip_prefix("(Hardware Port: ")
            .and_then(|line| line.strip_suffix(')'))
        else {
            continue;
        };
        let Some((hardware_port, device)) = details.split_once(", Device: ") else {
            pending = None;
            continue;
        };
        let Some((name, enabled)) = pending.take() else {
            continue;
        };
        let device = device.trim();
        if !device.is_empty() {
            services.push(MacosNetworkService {
                name,
                hardware_port: hardware_port.trim().to_string(),
                device: device.to_string(),
                enabled,
            });
        }
    }

    services
}

#[cfg(target_os = "macos")]
async fn run_networksetup_checked(args: &[&str]) -> Result<String, String> {
    let mut command = tokio::process::Command::new("networksetup");
    command.args(args);
    let output = run_cmd(command, TIMEOUT_MEDIUM).await?;
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    let reported_error = stdout
        .lines()
        .chain(stderr.lines())
        .find(|line| line.trim_start().starts_with("** Error:"));
    if output.status.success() && reported_error.is_none() {
        return Ok(stdout);
    }
    let detail = reported_error
        .map(str::trim)
        .filter(|detail| !detail.is_empty())
        .or_else(|| (!stderr.is_empty()).then_some(stderr.as_str()));
    Err(match detail {
        Some(detail) => format!("networksetup {} failed: {}", args.join(" "), detail),
        None => format!("networksetup {} failed", args.join(" ")),
    })
}

#[cfg(target_os = "macos")]
async fn list_macos_network_services() -> Result<Vec<MacosNetworkService>, String> {
    let text = run_networksetup_checked(&["-listnetworkserviceorder"]).await?;
    let services = parse_macos_network_service_order(&text);
    if services.is_empty() {
        Err("networksetup returned no physical network services".to_string())
    } else {
        Ok(services)
    }
}

#[cfg(target_os = "macos")]
fn is_macos_physical_uplink_device(iface: &str) -> bool {
    let Some(index) = iface.strip_prefix("en") else {
        return false;
    };
    !index.is_empty() && index.bytes().all(|byte| byte.is_ascii_digit())
}

#[cfg(target_os = "macos")]
async fn macos_service_for_device(iface: &str) -> Result<MacosNetworkService, String> {
    if !is_macos_physical_uplink_device(iface) {
        return Err(format!(
            "Refused macOS network mutation for non-physical interface {}. Only BSD enN uplinks are eligible; loopback, AWDL/LLW, bridges, VLANs, and tunnels are never cycled.",
            iface
        ));
    }
    let matches: Vec<MacosNetworkService> = list_macos_network_services()
        .await?
        .into_iter()
        .filter(|service| service.device == iface)
        .collect();

    match matches.as_slice() {
        [service] => Ok(service.clone()),
        [] => Err(format!(
            "No physical macOS network service maps to interface {}. Virtual/tunnel interfaces are never deep-reset.",
            iface
        )),
        _ => Err(format!(
            "Multiple macOS network services map to interface {}; refusing an ambiguous reset",
            iface
        )),
    }
}

#[cfg(target_os = "macos")]
async fn macos_service_enabled(service: &str) -> Result<bool, String> {
    let output = run_networksetup_checked(&["-getnetworkserviceenabled", service]).await?;
    match output.trim() {
        "Enabled" => Ok(true),
        "Disabled" => Ok(false),
        other => Err(format!(
            "Could not determine whether macOS network service \"{}\" is enabled (networksetup returned {:?})",
            service, other
        )),
    }
}

#[cfg(target_os = "macos")]
async fn set_macos_service_enabled(service: &str, enabled: bool) -> Result<(), String> {
    let state = if enabled { "on" } else { "off" };
    let mut command = tokio::process::Command::new("networksetup");
    command.args(["-setnetworkserviceenabled", service, state]);
    let output = run_macos_mutation(command, TIMEOUT_MEDIUM).await?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let reported_error = stdout
        .lines()
        .chain(stderr.lines())
        .find(|line| line.trim_start().starts_with("** Error:"));
    if output.status.success() && reported_error.is_none() {
        Ok(())
    } else {
        let detail = reported_error
            .map(str::trim)
            .filter(|detail| !detail.is_empty())
            .or_else(|| {
                let stderr = stderr.trim();
                (!stderr.is_empty()).then_some(stderr)
            });
        Err(match detail {
            Some(detail) => format!("networksetup -setnetworkserviceenabled failed: {}", detail),
            None => "networksetup -setnetworkserviceenabled failed".to_string(),
        })
    }
}

#[cfg(target_os = "macos")]
async fn wait_for_macos_service_state(service: &str, expected: bool) -> Result<(), String> {
    for _ in 0..5 {
        if macos_service_enabled(service).await? == expected {
            return Ok(());
        }
        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
    }
    Err(format!(
        "macOS network service \"{}\" did not become {}",
        service,
        if expected { "enabled" } else { "disabled" }
    ))
}

#[cfg(target_os = "macos")]
async fn macos_default_route_interface() -> Result<String, String> {
    let mut command = tokio::process::Command::new("route");
    command.args(["-n", "get", "default"]);
    let output = run_cmd(command, TIMEOUT_QUICK).await?;
    if !output.status.success() {
        return Err("macOS has no readable default route".to_string());
    }
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(str::trim)
        .find_map(|line| line.strip_prefix("interface:").map(str::trim))
        .filter(|interface| !interface.is_empty())
        .map(ToString::to_string)
        .ok_or_else(|| "macOS has no default-route interface".to_string())
}

#[cfg(target_os = "macos")]
async fn wait_for_macos_route_and_reachability(expected_iface: &str) -> Result<String, String> {
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(20);
    let mut last_route = None;
    while tokio::time::Instant::now() < deadline {
        if let Ok(route) = macos_default_route_interface().await {
            last_route = Some(route.clone());
            if route == expected_iface && super::connectivity::check_connectivity().await {
                return Ok(route);
            }
        }
        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
    }

    Err(match last_route {
        Some(route) if route != expected_iface => format!(
            "The network service is enabled, but the expected physical default route {} did not recover (current route: {})",
            expected_iface, route
        ),
        Some(route) => format!(
            "The network service is enabled and expected default route {} exists, but internet reachability did not recover",
            route
        ),
        None => "The network service is enabled, but no default route recovered".to_string(),
    })
}

#[cfg(target_os = "macos")]
async fn resolve_macos_cycle_restore_after_recovery(
    restore: &super::session::RestoreRegistry,
    token: super::session::RestoreToken,
    recovery: Result<String, String>,
) -> Result<String, String> {
    match recovery {
        Ok(route) => {
            restore.mark_resolved(token).await;
            Ok(route)
        }
        Err(error) => Err(error),
    }
}

/// Idempotent restore used by the interrupt registry. It never deletes or
/// recreates a service: it only re-enables the known service and restores the
/// exact DNS/search-domain values captured before the cycle.
#[cfg(target_os = "macos")]
pub(crate) async fn restore_macos_service_state(
    iface: &str,
    service: &str,
    dns_snapshot: &super::dns::MacosDnsSnapshot,
) -> Result<(), String> {
    set_macos_service_enabled(service, true).await?;
    wait_for_macos_service_state(service, true).await?;
    super::dns::restore_macos_dns_snapshot(dns_snapshot).await?;
    wait_for_macos_route_and_reachability(iface)
        .await
        .map(|_| ())
}

#[cfg(target_os = "macos")]
trait MacosServiceCycleOps {
    async fn set_service_enabled(&self, service: &str, enabled: bool) -> Result<(), String>;
    async fn wait_for_service_state(&self, service: &str, expected: bool) -> Result<(), String>;
    async fn capture_dns(&self, service: &str) -> Result<super::dns::MacosDnsSnapshot, String>;
    async fn restore_dns(&self, snapshot: &super::dns::MacosDnsSnapshot) -> Result<(), String>;
    async fn wait_for_route_and_reachability(&self, iface: &str) -> Result<String, String>;
    async fn delay(&self, duration: std::time::Duration);
}

#[cfg(target_os = "macos")]
struct SystemMacosServiceCycleOps;

#[cfg(target_os = "macos")]
impl MacosServiceCycleOps for SystemMacosServiceCycleOps {
    async fn set_service_enabled(&self, service: &str, enabled: bool) -> Result<(), String> {
        set_macos_service_enabled(service, enabled).await
    }

    async fn wait_for_service_state(&self, service: &str, expected: bool) -> Result<(), String> {
        wait_for_macos_service_state(service, expected).await
    }

    async fn capture_dns(&self, service: &str) -> Result<super::dns::MacosDnsSnapshot, String> {
        super::dns::capture_macos_dns_snapshot(service).await
    }

    async fn restore_dns(&self, snapshot: &super::dns::MacosDnsSnapshot) -> Result<(), String> {
        super::dns::restore_macos_dns_snapshot(snapshot).await
    }

    async fn wait_for_route_and_reachability(&self, iface: &str) -> Result<String, String> {
        wait_for_macos_route_and_reachability(iface).await
    }

    async fn delay(&self, duration: std::time::Duration) {
        tokio::time::sleep(duration).await;
    }
}

#[cfg(target_os = "macos")]
async fn execute_macos_service_cycle_transitions(
    ops: &impl MacosServiceCycleOps,
    restore: &super::session::RestoreRegistry,
    iface: &str,
    service: &MacosNetworkService,
    dns_snapshot: std::sync::Arc<super::dns::MacosDnsSnapshot>,
) -> Result<Vec<String>, String> {
    use super::session::RestoreOp;
    let token = restore
        .register(RestoreOp::ReEnableMacosService {
            iface: iface.to_string(),
            service: service.name.clone(),
            dns_snapshot: std::sync::Arc::clone(&dns_snapshot),
        })
        .await;

    let spinner = create_spinner(&format!(
        "Safely cycling network service \"{}\"...",
        service.name
    ));
    let disable_result = ops.set_service_enabled(&service.name, false).await;
    spinner.finish_and_clear();

    if let Err(error) = disable_result {
        return Err(format!(
            "Refused to continue after the service-disable command failed: {}. nd300 will verify/re-enable it while exiting.",
            error
        ));
    }

    ops.wait_for_service_state(&service.name, false).await?;

    ops.delay(std::time::Duration::from_secs(3)).await;

    if let Err(first_error) = ops.set_service_enabled(&service.name, true).await {
        ops.delay(std::time::Duration::from_secs(2)).await;
        if let Err(retry_error) = ops.set_service_enabled(&service.name, true).await {
            return Err(format!(
                "Network service \"{}\" is still disabled after two re-enable attempts ({}; retry: {}). nd300 will retry during cleanup. Manual command: sudo networksetup -setnetworkserviceenabled \"{}\" on",
                service.name, first_error, retry_error, service.name
            ));
        }
    }
    ops.wait_for_service_state(&service.name, true).await?;

    let current_dns = ops.capture_dns(&service.name).await?;
    if current_dns != *dns_snapshot {
        if let Err(error) = ops.restore_dns(&dns_snapshot).await {
            return Err(format!(
                "The service cycle changed DNS/search domains and exact restoration failed: {}. nd300 will retry during cleanup.",
                error
            ));
        }
        return Err(
            "The service cycle unexpectedly changed DNS/search domains; the exact prior values were restored and the reset was stopped."
                .to_string(),
        );
    }

    // Keep the idempotent restore pending until enabled state, exact resolver
    // state, the expected physical default route, and internet reachability
    // have all been verified. A route/reachability failure therefore still
    // goes through the LIFO cleanup drain.
    let recovery = ops.wait_for_route_and_reachability(iface).await;
    let route = resolve_macos_cycle_restore_after_recovery(restore, token, recovery).await?;
    Ok(vec![
        format!(
            "Safely cycled service \"{}\" ({}, {})",
            service.name, service.hardware_port, iface
        ),
        format!("Verified default route {} and internet reachability", route),
    ])
}

#[cfg(target_os = "macos")]
async fn execute_macos_service_cycle(
    restore: &super::session::RestoreRegistry,
) -> Result<Vec<String>, String> {
    let iface = adapters::detect_default_interface()
        .await
        .ok_or_else(|| "Could not detect the default interface".to_string())?;
    let service = macos_service_for_device(&iface).await?;
    classify_macos_hardware_port(&service.hardware_port).ok_or_else(|| {
        format!(
            "Refused to cycle {} because hardware port {:?} could not be classified safely as Wi-Fi or Ethernet",
            iface, service.hardware_port
        )
    })?;

    if !service.enabled || !macos_service_enabled(&service.name).await? {
        return Err(format!(
            "macOS network service \"{}\" is disabled; nd300 will not alter an intentionally disabled service",
            service.name
        ));
    }

    // The exact resolver snapshot is a precondition, not best effort. No
    // mutation or restore registration occurs if either DNS read fails.
    let dns_snapshot =
        std::sync::Arc::new(super::dns::capture_macos_dns_snapshot(&service.name).await?);
    execute_macos_service_cycle_transitions(
        &SystemMacosServiceCycleOps,
        restore,
        &iface,
        &service,
        dns_snapshot,
    )
    .await
}

#[cfg(target_os = "macos")]
async fn stage3_macos(
    _config: &Config,
    _saved_ssid: &Option<String>,
    restore: &super::session::RestoreRegistry,
) -> Result<Vec<String>, String> {
    // The implementation below is deliberately shipped fail-closed until it
    // has passed a disposable macOS service/VM mutation test. The active
    // developer Mac must never be used to validate a high-risk reset.
    if !MACOS_SERVICE_CYCLE_VALIDATED {
        return Err(
            "macOS deep reset is safely unavailable in this build: the former service-deletion reset was removed after it could damage Wi-Fi configuration. Use the lower-risk adapter restart, or re-add/repair the service in System Settings with a backup. The replacement service-cycle remains gated until disposable-Mac validation."
                .to_string(),
        );
    }

    execute_macos_service_cycle(restore).await
}

/// Resolve a BSD device such as `en0` to its actual, user-renamable macOS
/// network service. Hardware-port display names are intentionally not returned.
#[cfg(target_os = "macos")]
pub async fn detect_macos_service(iface: &str) -> Option<String> {
    macos_service_for_device(iface)
        .await
        .ok()
        .map(|service| service.name)
}

#[cfg(all(test, target_os = "macos"))]
mod macos_service_cycle_tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};

    const LIVE_CYCLE_ACK_VAR: &str = "ND300_LIVE_MACOS_SERVICE_CYCLE_ACK";
    const LIVE_CYCLE_ACK: &str =
        "I_UNDERSTAND_ND300_WILL_BRIEFLY_DISABLE_MY_ACTIVE_MACOS_NETWORK_SERVICE";
    const LIVE_WATCHDOG_PID_VAR: &str = "ND300_LIVE_MACOS_SERVICE_CYCLE_WATCHDOG_PID";
    const LIVE_EXPECTED_INTERFACE_VAR: &str = "ND300_LIVE_MACOS_SERVICE_CYCLE_EXPECTED_INTERFACE";
    const LIVE_EXPECTED_SERVICE_VAR: &str = "ND300_LIVE_MACOS_SERVICE_CYCLE_EXPECTED_SERVICE";

    const MODERN_MACOS: &str = r#"
An asterisk (*) denotes that a network service is disabled.
(1) USB 10/100/1000 LAN
(Hardware Port: USB 10/100/1000 LAN, Device: en5)

(2) Studio Wi-Fi
(Hardware Port: Wi-Fi, Device: en0)

(*) Disabled Bridge
(Hardware Port: Thunderbolt Bridge, Device: bridge0)

(4) Tailscale
(Hardware Port: io.tailscale.ipn.macos, Device: )
"#;

    #[test]
    fn service_order_maps_device_to_actual_renamed_service() {
        let services = parse_macos_network_service_order(MODERN_MACOS);
        let wifi = services
            .iter()
            .find(|service| service.device == "en0")
            .unwrap();
        assert_eq!(wifi.name, "Studio Wi-Fi");
        assert_eq!(wifi.hardware_port, "Wi-Fi");
        assert!(wifi.enabled);

        let bridge = services
            .iter()
            .find(|service| service.device == "bridge0")
            .unwrap();
        assert!(!bridge.enabled);
        assert!(
            services.iter().all(|service| !service.device.is_empty()),
            "virtual services without a physical device must not be reset candidates"
        );
        assert!(is_macos_physical_uplink_device("en0"));
        assert!(is_macos_physical_uplink_device("en15"));
        for virtual_iface in ["lo0", "awdl0", "llw0", "bridge0", "vlan0", "utun3"] {
            assert!(
                !is_macos_physical_uplink_device(virtual_iface),
                "{virtual_iface} must never be a service-cycle candidate"
            );
        }
        assert_eq!(classify_macos_hardware_port("Wi-Fi"), Some(true));
        assert_eq!(classify_macos_hardware_port("AirPort"), Some(true));
        assert_eq!(
            classify_macos_hardware_port("USB 10/100/1000 LAN"),
            Some(false)
        );
        assert_eq!(
            classify_macos_hardware_port("Thunderbolt Ethernet Slot 1"),
            Some(false)
        );
        assert_eq!(classify_macos_hardware_port("Mystery Port"), None);
    }

    #[test]
    fn adapter_recovery_parsers_require_positive_operational_evidence() {
        assert_eq!(
            parse_macos_airport_power("Wi-Fi Power (en0): On\n"),
            Some(true)
        );
        assert_eq!(
            parse_macos_airport_power("Wi-Fi Power (en0): Off\n"),
            Some(false)
        );
        assert_eq!(parse_macos_airport_power("unknown\n"), None);

        assert!(parse_macos_ifconfig_operational(
            "en0: flags=8863<UP,BROADCAST,RUNNING,SIMPLEX,MULTICAST> mtu 1500\n\tinet 10.0.0.2 netmask 0xffffff00\n\tstatus: active\n"
        ));
        assert!(!parse_macos_ifconfig_operational(
            "en0: flags=8862<BROADCAST,RUNNING,SIMPLEX,MULTICAST> mtu 1500\n\tstatus: inactive\n"
        ));
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum InjectedFault {
        None,
        DisableCommand,
        DisabledStateVerification,
        FirstEnableCommand,
        BothEnableCommands,
        EnabledStateVerification,
        PostCycleDnsRead,
        DnsChangedButRestored,
        DnsRestore,
        RouteOrReachability,
    }

    struct FaultInjectingCycleOps {
        fault: InjectedFault,
        restore: super::super::session::RestoreRegistry,
        original_dns: super::super::dns::MacosDnsSnapshot,
        changed_dns: super::super::dns::MacosDnsSnapshot,
        enable_attempts: AtomicUsize,
        registered_before_disable: AtomicBool,
        calls: Mutex<Vec<&'static str>>,
    }

    impl FaultInjectingCycleOps {
        fn record(&self, call: &'static str) {
            self.calls.lock().unwrap().push(call);
        }
    }

    impl MacosServiceCycleOps for FaultInjectingCycleOps {
        async fn set_service_enabled(&self, _service: &str, enabled: bool) -> Result<(), String> {
            if enabled {
                self.record("enable");
                let attempt = self.enable_attempts.fetch_add(1, Ordering::SeqCst);
                if self.fault == InjectedFault::BothEnableCommands
                    || (self.fault == InjectedFault::FirstEnableCommand && attempt == 0)
                {
                    return Err("injected enable failure".to_string());
                }
            } else {
                self.record("disable");
                self.registered_before_disable
                    .store(self.restore.pending_count().await == 1, Ordering::SeqCst);
                if self.fault == InjectedFault::DisableCommand {
                    return Err("injected disable failure".to_string());
                }
            }
            Ok(())
        }

        async fn wait_for_service_state(
            &self,
            _service: &str,
            expected: bool,
        ) -> Result<(), String> {
            if expected {
                self.record("wait-enabled");
                if self.fault == InjectedFault::EnabledStateVerification {
                    return Err("injected enabled-state verification failure".to_string());
                }
            } else {
                self.record("wait-disabled");
                if self.fault == InjectedFault::DisabledStateVerification {
                    return Err("injected disabled-state verification failure".to_string());
                }
            }
            Ok(())
        }

        async fn capture_dns(
            &self,
            _service: &str,
        ) -> Result<super::super::dns::MacosDnsSnapshot, String> {
            self.record("capture-dns");
            if self.fault == InjectedFault::PostCycleDnsRead {
                return Err("injected post-cycle DNS read failure".to_string());
            }
            if matches!(
                self.fault,
                InjectedFault::DnsChangedButRestored | InjectedFault::DnsRestore
            ) {
                Ok(self.changed_dns.clone())
            } else {
                Ok(self.original_dns.clone())
            }
        }

        async fn restore_dns(
            &self,
            _snapshot: &super::super::dns::MacosDnsSnapshot,
        ) -> Result<(), String> {
            self.record("restore-dns");
            if self.fault == InjectedFault::DnsRestore {
                Err("injected DNS restore failure".to_string())
            } else {
                Ok(())
            }
        }

        async fn wait_for_route_and_reachability(&self, _iface: &str) -> Result<String, String> {
            self.record("route");
            if self.fault == InjectedFault::RouteOrReachability {
                Err("injected route/reachability failure".to_string())
            } else {
                Ok("en0".to_string())
            }
        }

        async fn delay(&self, _duration: std::time::Duration) {
            self.record("delay");
        }
    }

    async fn run_injected_cycle(
        fault: InjectedFault,
    ) -> (Result<Vec<String>, String>, usize, bool, Vec<&'static str>) {
        let restore = super::super::session::RestoreRegistry::new();
        let original_dns = super::super::dns::MacosDnsSnapshot::for_test(
            "Studio Wi-Fi",
            &["1.1.1.1", "1.0.0.1"],
            &["example.test"],
        );
        let ops = FaultInjectingCycleOps {
            fault,
            restore: restore.clone(),
            changed_dns: super::super::dns::MacosDnsSnapshot::for_test(
                "Studio Wi-Fi",
                &["8.8.8.8"],
                &[],
            ),
            original_dns: original_dns.clone(),
            enable_attempts: AtomicUsize::new(0),
            registered_before_disable: AtomicBool::new(false),
            calls: Mutex::new(Vec::new()),
        };
        let service = MacosNetworkService {
            name: "Studio Wi-Fi".to_string(),
            hardware_port: "Wi-Fi".to_string(),
            device: "en0".to_string(),
            enabled: true,
        };

        let result = execute_macos_service_cycle_transitions(
            &ops,
            &restore,
            "en0",
            &service,
            Arc::new(original_dns),
        )
        .await;
        let pending = restore.pending_count().await;
        let registered = ops.registered_before_disable.load(Ordering::SeqCst);
        let calls = ops.calls.into_inner().unwrap();
        (result, pending, registered, calls)
    }

    #[tokio::test]
    async fn every_service_cycle_transition_fails_closed_with_restore_pending() {
        let cases = [
            (InjectedFault::DisableCommand, "disable"),
            (InjectedFault::DisabledStateVerification, "wait-disabled"),
            (InjectedFault::BothEnableCommands, "enable"),
            (InjectedFault::EnabledStateVerification, "wait-enabled"),
            (InjectedFault::PostCycleDnsRead, "capture-dns"),
            (InjectedFault::DnsChangedButRestored, "restore-dns"),
            (InjectedFault::DnsRestore, "restore-dns"),
            (InjectedFault::RouteOrReachability, "route"),
        ];

        for (fault, expected_call) in cases {
            let (result, pending, registered, calls) = run_injected_cycle(fault).await;
            assert!(result.is_err(), "{fault:?} unexpectedly succeeded");
            assert!(
                registered,
                "{fault:?} reached mutation before registering its inverse"
            );
            assert_eq!(pending, 1, "{fault:?} incorrectly resolved its restore");
            assert!(
                calls.contains(&expected_call),
                "{fault:?} did not reach expected transition {expected_call}: {calls:?}"
            );
        }
    }

    #[tokio::test]
    async fn first_enable_failure_retries_and_only_full_success_resolves_restore() {
        let (retry_result, retry_pending, retry_registered, retry_calls) =
            run_injected_cycle(InjectedFault::FirstEnableCommand).await;
        assert!(retry_result.is_ok());
        assert!(retry_registered);
        assert_eq!(retry_pending, 0);
        assert_eq!(
            retry_calls.iter().filter(|call| **call == "enable").count(),
            2
        );

        let (success, pending, registered, calls) = run_injected_cycle(InjectedFault::None).await;
        assert!(success.is_ok());
        assert!(registered);
        assert_eq!(pending, 0);
        assert_eq!(calls.last(), Some(&"route"));
    }

    #[tokio::test]
    async fn gated_deep_reset_returns_before_registering_or_mutating() {
        let restore = super::super::session::RestoreRegistry::new();
        let result = stage3_macos(&Config::new(), &None, &restore).await;
        assert!(result.unwrap_err().contains("safely unavailable"));
        assert_eq!(restore.pending_count().await, 0);
    }

    #[tokio::test]
    async fn restore_token_resolves_only_after_full_recovery_verification() {
        use super::super::session::RestoreOp;

        let failed = super::super::session::RestoreRegistry::new();
        let failed_token = failed
            .register(RestoreOp::ReEnableInterface {
                iface: "test-only".to_string(),
            })
            .await;
        assert!(resolve_macos_cycle_restore_after_recovery(
            &failed,
            failed_token,
            Err("wrong route".to_string()),
        )
        .await
        .is_err());
        assert_eq!(failed.pending_count().await, 1);

        let recovered = super::super::session::RestoreRegistry::new();
        let recovered_token = recovered
            .register(RestoreOp::ReEnableInterface {
                iface: "test-only".to_string(),
            })
            .await;
        assert_eq!(
            resolve_macos_cycle_restore_after_recovery(
                &recovered,
                recovered_token,
                Ok("en0".to_string()),
            )
            .await
            .unwrap(),
            "en0"
        );
        assert_eq!(recovered.pending_count().await, 0);
    }

    /// Destructive acceptance entrypoint for a deliberately supervised native
    /// Mac run. This is test-only, ignored by default, and calls the exact
    /// private production implementation below the release gate. It is not a
    /// CLI escape hatch.
    ///
    /// The caller must run the already-built test binary as root under an
    /// independent, offline-capable watchdog that can re-enable the captured
    /// service if this process is killed or wedges. Both the exact
    /// acknowledgement and the live watchdog PID are required to make an
    /// accidental `cargo test -- --ignored` invocation fail before mutation.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    #[ignore = "requires explicit root acknowledgement and an independent offline watchdog"]
    async fn live_exact_service_cycle_restores_all_captured_state() {
        use futures_util::FutureExt;

        assert_eq!(
            std::env::var(LIVE_CYCLE_ACK_VAR).as_deref(),
            Ok(LIVE_CYCLE_ACK),
            "refusing live service cycle: set {LIVE_CYCLE_ACK_VAR} to the documented exact acknowledgement"
        );
        // SAFETY: geteuid has no preconditions and does not dereference
        // pointers.
        assert_eq!(
            unsafe { libc::geteuid() },
            0,
            "refusing live service cycle: run the already-built test binary as root"
        );

        let watchdog_pid = std::env::var(LIVE_WATCHDOG_PID_VAR)
            .ok()
            .and_then(|value| value.parse::<libc::pid_t>().ok())
            .filter(|pid| *pid > 1 && u32::try_from(*pid).ok() != Some(std::process::id()))
            .expect(
                "refusing live service cycle: provide the independent watchdog PID in \
                 ND300_LIVE_MACOS_SERVICE_CYCLE_WATCHDOG_PID",
            );
        // Signal zero performs existence/permission validation without sending
        // a signal. Root should be able to inspect the independent watchdog.
        // SAFETY: kill with signal 0 has no pointer arguments and does not
        // mutate the target process.
        assert_eq!(
            unsafe { libc::kill(watchdog_pid, 0) },
            0,
            "refusing live service cycle: watchdog PID {watchdog_pid} is not alive"
        );

        let iface = adapters::detect_default_interface()
            .await
            .expect("live acceptance requires a default interface");
        let service = macos_service_for_device(&iface)
            .await
            .expect("live acceptance requires one unambiguous physical network service");
        assert_eq!(
            std::env::var(LIVE_EXPECTED_INTERFACE_VAR).as_deref(),
            Ok(iface.as_str()),
            "refusing live service cycle: expected interface acknowledgement does not match the mapped default interface"
        );
        assert_eq!(
            std::env::var(LIVE_EXPECTED_SERVICE_VAR).as_deref(),
            Ok(service.name.as_str()),
            "refusing live service cycle: expected service acknowledgement does not match the mapped default service"
        );
        assert!(
            service.enabled && macos_service_enabled(&service.name).await.unwrap_or(false),
            "refusing live service cycle: selected service is intentionally disabled"
        );
        let before_dns = super::super::dns::capture_macos_dns_snapshot(&service.name)
            .await
            .expect("live acceptance could not capture exact DNS/search-domain state");

        let restore = super::super::session::RestoreRegistry::new();
        let cycle = std::panic::AssertUnwindSafe(execute_macos_service_cycle(&restore))
            .catch_unwind()
            .await;

        // Always attempt cleanup before interpreting the cycle result, even if
        // the production future panicked. This supplements rather than replaces
        // the independent watchdog.
        let cleanup_failures =
            tokio::time::timeout(std::time::Duration::from_secs(90), restore.drain())
                .await
                .expect("live service-cycle cleanup exceeded 90 seconds");
        assert!(
            cleanup_failures.is_empty(),
            "live service-cycle cleanup failed: {}",
            cleanup_failures.join("; ")
        );

        assert!(
            macos_service_enabled(&service.name)
                .await
                .expect("could not verify final service state"),
            "live service-cycle left the selected service disabled"
        );
        let after_dns = super::super::dns::capture_macos_dns_snapshot(&service.name)
            .await
            .expect("could not verify final DNS/search-domain state");
        assert_eq!(
            after_dns, before_dns,
            "live service-cycle did not preserve exact DNS/search-domain state"
        );
        wait_for_macos_route_and_reachability(&iface)
            .await
            .expect("live service-cycle did not recover its physical route and reachability");
        assert_eq!(
            restore.pending_count().await,
            0,
            "live service-cycle left unresolved cleanup operations"
        );

        match cycle {
            Ok(Ok(steps)) => assert!(
                !steps.is_empty(),
                "live service-cycle returned no completed verification steps"
            ),
            Ok(Err(error)) => panic!("live production service-cycle failed safely: {error}"),
            Err(payload) => std::panic::resume_unwind(payload),
        }
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
        return Err(
            "Linux deep reset is safely unavailable while NetworkManager controls the active connection. The former profile deletion/recreation path could leave the profile permanently missing if the process was interrupted. Use the lower-risk adapter restart or DHCP renewal, or repair the profile in NetworkManager settings."
                .to_string(),
        );
    }

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
    let _ = super::cmd::run_owned_mutation(restart_cmd, TIMEOUT_MEDIUM).await;
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
            let _ = super::cmd::run_owned_mutation(reconf_cmd, TIMEOUT_QUICK).await;
            completed.push(format!(
                "Connected to {} via wpa_supplicant",
                ssid_to_connect
            ));
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
