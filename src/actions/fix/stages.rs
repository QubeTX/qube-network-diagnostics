use crate::config::Config;
use crate::render::progress::create_spinner;

#[allow(unused_imports)]
use super::cmd::{run_cmd, TIMEOUT_MEDIUM, TIMEOUT_QUICK, TIMEOUT_SLOW};
#[cfg(any(windows, target_os = "linux"))]
use super::wifi;
#[cfg(target_os = "linux")]
use crate::actions::prompt_string;
use crate::actions::{is_interactive, prompt_yes_no};

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

/// Snapshot of a macOS network service's settings, captured before the deep
/// reset removes the service so they can be restored after recreation.
///
/// `pub` (matching the public `super::session::RestoreOp` that carries it, like
/// the sibling `pub` `DisabledVpn`) so it doesn't trip the `private_interfaces`
/// lint as a more-private type behind a public enum field — this only surfaces on
/// macOS (the variant is `#[cfg(target_os = "macos")]`) and only under
/// `-D warnings` (so it slipped past the macOS release build until the
/// cross-platform `ci.yml`). Fields stay private — only this module constructs,
/// captures, and restores it.
#[cfg(target_os = "macos")]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MacosNetworkSnapshot {
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

/// Create (or recreate) a macOS network service for the given interface. One
/// attempt; returns `Ok(())` on success. Extracted so the deep reset and the
/// restore-registry drain share exactly one create path.
#[cfg(target_os = "macos")]
async fn create_macos_service(service: &str, iface: &str) -> Result<(), String> {
    let mut cmd = tokio::process::Command::new("networksetup");
    cmd.args(["-createnetworkservice", service, iface]);
    match run_cmd(cmd, TIMEOUT_MEDIUM).await {
        Ok(output) if output.status.success() => Ok(()),
        Ok(output) => {
            let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
            if stderr.is_empty() {
                Err(format!("Failed to recreate service: {}", service))
            } else {
                Err(format!(
                    "Failed to recreate service {}: {}",
                    service, stderr
                ))
            }
        }
        Err(e) => Err(e),
    }
}

/// Recreate a removed macOS network service and restore its captured settings.
/// Used by the restore-registry drain when a deep reset is interrupted (or its
/// own recreate failed). Retries the create once, then best-effort restores the
/// snapshot. Returns `Err` only when the service still cannot be recreated —
/// that is the case where the Mac is genuinely left without the service and the
/// user needs manual recovery.
#[cfg(target_os = "macos")]
pub(crate) async fn recreate_and_restore_macos_service(
    iface: &str,
    service: &str,
    snapshot: &MacosNetworkSnapshot,
) -> Result<(), String> {
    // Retry create once — leaving the Mac with no network service is the worst
    // possible outcome.
    if let Err(first) = create_macos_service(service, iface).await {
        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
        if let Err(retry) = create_macos_service(service, iface).await {
            return Err(format!(
                "could not recreate network service \"{}\" for {} ({}; retry: {}). \
                 Open System Settings ▸ Network and re-add the service manually",
                service, iface, first, retry
            ));
        }
    }
    // Service exists again — best-effort restore of its previous settings.
    let _ = restore_macos_network_snapshot(snapshot).await;
    Ok(())
}

#[cfg(target_os = "macos")]
async fn stage3_macos(
    config: &Config,
    saved_ssid: &Option<String>,
    restore: &super::session::RestoreRegistry,
) -> Result<Vec<String>, String> {
    use super::session::RestoreOp;

    let mut completed = Vec::new();

    // Detect default interface and its network service name
    let iface = adapters::detect_default_interface()
        .await
        .ok_or_else(|| "Could not detect default interface".to_string())?;

    let service_name = detect_macos_service(&iface)
        .await
        .ok_or_else(|| format!("Could not find network service for {}", iface))?;
    let snapshot = capture_macos_network_snapshot(&service_name).await;

    // Register the recreate-and-restore op BEFORE removing the service, so any
    // interrupt between remove and recreate (or a failed recreate) leaves the
    // drain holding a job that puts the service back.
    let token = restore
        .register(RestoreOp::RecreateMacosService {
            iface: iface.clone(),
            service: service_name.clone(),
            snapshot: snapshot.clone(),
        })
        .await;

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
            _ => {
                // The remove never succeeded — the service is still present, so
                // there's nothing to restore. Resolve the op and bail.
                restore.mark_resolved(token).await;
                return Err(format!("Failed to remove service: {}", service_name));
            }
        }
    }

    // 2 + 3. Recreate the service and restore its previous settings via the
    // shared helper (retries create once). This avoids converting a
    // DHCP/default-DNS service into a hard-coded public-DNS service.
    {
        let spinner = create_spinner(&format!("Recreating service \"{}\"...", service_name));
        let result = recreate_and_restore_macos_service(&iface, &service_name, &snapshot).await;
        spinner.finish_and_clear();
        match result {
            Ok(()) => {
                // Service is back and settings restored — resolve so the drain
                // won't repeat the work.
                restore.mark_resolved(token).await;
                completed.push(format!("Recreated service: {}", service_name));
                completed.push("Restored previous macOS network service settings".to_string());
            }
            Err(e) => {
                // Leave the op REGISTERED so the terminal drain retries it, and
                // return Err with manual-recovery text.
                return Err(format!(
                    "Network service \"{}\" was removed but could not be recreated: {}. \
                     nd300 will try again as it exits.",
                    service_name, e
                ));
            }
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

        // Delete active connection profile. Check the result before claiming
        // success — if the delete failed, the old (possibly broken) profile is
        // still present, so recreating on top of it would be misleading.
        if let Some(profile) = detect_nm_profile(&iface).await {
            let spinner = create_spinner(&format!("Deleting profile \"{}\"...", profile));
            let mut cmd = tokio::process::Command::new("nmcli");
            cmd.args(["connection", "delete", &profile]);
            let r = run_cmd(cmd, TIMEOUT_MEDIUM).await;
            spinner.finish_and_clear();
            match r {
                Ok(output) if output.status.success() => {
                    completed.push(format!("Deleted profile: {}", profile));
                }
                Ok(output) => {
                    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
                    return Err(format!(
                        "Failed to delete connection profile \"{}\"{}",
                        profile,
                        if stderr.is_empty() {
                            String::new()
                        } else {
                            format!(": {}", stderr)
                        }
                    ));
                }
                Err(e) => {
                    return Err(format!(
                        "Failed to delete connection profile \"{}\": {}",
                        profile, e
                    ));
                }
            }
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
