use crate::config::Config;
use crate::diagnostics::vpn::{self, VpnAdapter};
use crate::render::progress::create_spinner;

#[cfg(windows)]
use super::cmd::TIMEOUT_SLOW;
use super::cmd::{run_cmd, run_owned_mutation, TIMEOUT_MEDIUM};
use super::{print_step_fail, print_step_ok, warn_icon};
use crate::actions::{is_interactive, prompt_yes_no};

/// Info about a VPN that was disabled during the fix, for potential re-enable.
///
/// `Clone` so a copy can be stowed in the restore registry (wrapped in `Arc`)
/// while the original list is still passed to [`offer_reenable`].
#[derive(Debug, Clone)]
pub struct DisabledVpn {
    pub name: String,
    pub method: DisableMethod,
}

#[derive(Debug, Clone)]
pub enum DisableMethod {
    VendorCli(String, Vec<String>), // (binary, args)
    Netsh(String),                  // adapter name (Windows)
    #[cfg(target_os = "macos")]
    Scutil(String), // VPN service name
    #[cfg(target_os = "linux")]
    Nmcli(String), // connection name
    #[cfg(target_os = "linux")]
    WgQuick(String), // interface name
}

/// Reconnect commands commonly return before the VPN has finished changing
/// state. Keep the mutation itself bounded, then require a separate bounded
/// readback before treating a restore as complete.
const VPN_MUTATION_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(15);
const VPN_STATE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(3);
const VPN_STATE_ATTEMPTS: usize = 6;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum VpnConnectionState {
    Connected,
    Disconnected,
    Unknown,
}

/// Known enterprise VPN vendors — we never touch these automatically.
fn is_enterprise_vpn(adapter: &VpnAdapter) -> bool {
    let lower = adapter.name.to_lowercase();
    let vendor_lower = adapter.vendor.as_deref().unwrap_or("").to_lowercase();
    let type_lower = adapter.adapter_type.to_lowercase();

    let enterprise_patterns = [
        "cisco",
        "anyconnect",
        "globalprotect",
        "palo alto",
        "zscaler",
        "forticlient",
        "fortinet",
        "pulse secure",
        "juniper",
        "f5 ",
        "big-ip",
        "checkpoint",
        "corp",
        "enterprise",
        "mdm",
        "company",
    ];

    enterprise_patterns
        .iter()
        .any(|p| lower.contains(p) || vendor_lower.contains(p) || type_lower.contains(p))
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
            return Some((
                "wg-quick".to_string(),
                vec!["down".to_string(), iface.clone()],
            ));
        }
    }

    // Cisco AnyConnect
    if lower.contains("cisco")
        || vendor_lower.contains("cisco")
        || adapter.adapter_type.contains("Cisco")
    {
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
            return Some((
                "/opt/cisco/anyconnect/bin/vpn".to_string(),
                vec!["disconnect".to_string()],
            ));
        }
    }

    None
}

/// A VPN successfully disabled only after its inverse operation was registered.
#[derive(Debug)]
pub struct GuardedDisabledVpn {
    pub vpn: DisabledVpn,
    pub restore_token: super::session::RestoreToken,
}

#[derive(Debug)]
enum DisableAttempt {
    Disabled(GuardedDisabledVpn),
    /// The process could not be spawned, so it definitely made no change and
    /// its inverse was safely resolved.
    DefinitelyNotRun,
    /// The command timed out or exited non-zero after spawning. It may still
    /// have disconnected the VPN, so its inverse remains registered.
    Uncertain,
}

enum DisconnectRun {
    Completed(std::process::ExitStatus),
    SpawnFailed,
    TimedOut,
}

async fn run_disconnect_command(
    command: tokio::process::Command,
    timeout: std::time::Duration,
) -> DisconnectRun {
    // Use the same serialized child-owning runner as reconnect. If this future
    // is cancelled, its supervisor keeps the mutation lock until the spawned
    // disconnect has been killed and reaped; cleanup reconnect cannot race it.
    match run_owned_mutation(command, timeout).await {
        Ok(output) => DisconnectRun::Completed(output.status),
        Err(error) if error.contains(" spawn failed:") => DisconnectRun::SpawnFailed,
        Err(_) => DisconnectRun::TimedOut,
    }
}

#[derive(Debug, Default)]
pub struct VpnDisableBatch {
    pub disabled: Vec<GuardedDisabledVpn>,
    pub uncertain_attempts: usize,
}

async fn run_registered_disable(
    vpn: DisabledVpn,
    command: tokio::process::Command,
    timeout: std::time::Duration,
    config: &Config,
    restore: &super::session::RestoreRegistry,
) -> DisableAttempt {
    use super::session::RestoreOp;
    use std::sync::Arc;

    // Register the inverse before spawning the disconnect command. Cancellation,
    // timeout, panic, SIGINT, SIGTERM, or SIGHUP can never land in a gap where a
    // VPN has been disconnected but no reconnect operation exists.
    let restore_token = restore
        .register(RestoreOp::ReEnableVpn(Arc::new(vpn.clone())))
        .await;
    let result = run_disconnect_command(command, timeout).await;
    match result {
        DisconnectRun::Completed(status) if status.success() => {
            if is_interactive(config) {
                print_step_ok(&format!("Disabled {}", vpn.name), config);
            }
            DisableAttempt::Disabled(GuardedDisabledVpn { vpn, restore_token })
        }
        DisconnectRun::SpawnFailed => {
            // A spawn failure means no child existed and no disconnect could
            // have happened. Only this case is safe to resolve before trying
            // another strategy.
            restore.mark_resolved(restore_token).await;
            DisableAttempt::DefinitelyNotRun
        }
        // A spawned command can perform the disconnect and still exit nonzero,
        // or outlive our timeout. Keep the inverse pending; idempotent reconnect
        // during cleanup is safer than stranding the VPN.
        DisconnectRun::Completed(_) | DisconnectRun::TimedOut => DisableAttempt::Uncertain,
    }
}

/// Detect connected VPNs and prompt user to disable them before fix stages.
/// Every returned VPN had its reconnect operation registered before the
/// disconnect subprocess was spawned.
pub async fn detect_and_disable(
    config: &Config,
    restore: &super::session::RestoreRegistry,
) -> VpnDisableBatch {
    let mut batch = VpnDisableBatch::default();

    let spinner = create_spinner("Detecting VPN connections...");
    let vpns = vpn::collect().await;
    spinner.finish_and_clear();

    let vpns = match vpns {
        Some(v) => v,
        None => return batch,
    };

    let connected: Vec<&VpnAdapter> = vpns.iter().filter(|v| v.status == "Connected").collect();
    if connected.is_empty() {
        return batch;
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
            false
        };

        if !do_disable {
            continue;
        }

        // Try a vendor CLI first, but guard it before spawning.
        if let Some((bin, args)) = find_vendor_cli(adapter) {
            let guarded = DisabledVpn {
                name: adapter.name.clone(),
                method: DisableMethod::VendorCli(
                    bin.clone(),
                    args.iter()
                        .map(|arg| match arg.as_str() {
                            "disconnect" => "connect".to_string(),
                            "down" => "up".to_string(),
                            _ => arg.clone(),
                        })
                        .collect(),
                ),
            };
            let mut command = tokio::process::Command::new(&bin);
            command.args(&args);
            let spinner = create_spinner(&format!("Disabling {}...", adapter.name));
            let result =
                run_registered_disable(guarded, command, TIMEOUT_MEDIUM, config, restore).await;
            spinner.finish_and_clear();
            match result {
                DisableAttempt::Disabled(result) => {
                    batch.disabled.push(result);
                    continue;
                }
                DisableAttempt::Uncertain => {
                    batch.uncertain_attempts += 1;
                    if is_interactive(config) {
                        print_step_fail(
                            &format!("Could not confirm {} disconnected", adapter.name),
                            "ND300 will conservatively reconnect it during cleanup",
                            config,
                        );
                    }
                    continue;
                }
                DisableAttempt::DefinitelyNotRun => {}
            }
        }

        let spinner = create_spinner(&format!("Disabling {}...", adapter.name));
        let fallback_result = disable_adapter_fallback(adapter, config, restore).await;
        spinner.finish_and_clear();
        match fallback_result {
            DisableAttempt::Disabled(disabled_vpn) => batch.disabled.push(disabled_vpn),
            DisableAttempt::Uncertain => {
                batch.uncertain_attempts += 1;
                if is_interactive(config) {
                    print_step_fail(
                        &format!("Could not confirm {} disconnected", adapter.name),
                        "ND300 will conservatively reconnect it during cleanup",
                        config,
                    );
                }
            }
            DisableAttempt::DefinitelyNotRun => {
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

    if !batch.disabled.is_empty() || batch.uncertain_attempts > 0 {
        let spinner = create_spinner("Waiting for VPN disconnect...");
        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
        spinner.finish_and_clear();
    }

    batch
}

async fn disable_adapter_fallback(
    adapter: &VpnAdapter,
    config: &Config,
    restore: &super::session::RestoreRegistry,
) -> DisableAttempt {
    #[cfg(windows)]
    {
        if let Some(ref iface) = adapter.interface_name {
            let vpn = DisabledVpn {
                name: adapter.name.clone(),
                method: DisableMethod::Netsh(iface.clone()),
            };
            let mut command = tokio::process::Command::new("netsh");
            command.args(["interface", "set", "interface", iface, "disabled"]);
            return run_registered_disable(vpn, command, TIMEOUT_SLOW, config, restore).await;
        }
        DisableAttempt::DefinitelyNotRun
    }

    #[cfg(target_os = "macos")]
    {
        let vpn = DisabledVpn {
            name: adapter.name.clone(),
            method: DisableMethod::Scutil(adapter.name.clone()),
        };
        let mut command = tokio::process::Command::new("scutil");
        command.args(["--nc", "stop", &adapter.name]);
        run_registered_disable(vpn, command, TIMEOUT_MEDIUM, config, restore).await
    }

    #[cfg(target_os = "linux")]
    {
        let nmcli_vpn = DisabledVpn {
            name: adapter.name.clone(),
            method: DisableMethod::Nmcli(adapter.name.clone()),
        };
        let mut nmcli = tokio::process::Command::new("nmcli");
        nmcli.args(["connection", "down", &adapter.name]);
        match run_registered_disable(nmcli_vpn, nmcli, TIMEOUT_MEDIUM, config, restore).await {
            result @ DisableAttempt::Disabled(_) | result @ DisableAttempt::Uncertain => {
                return result;
            }
            DisableAttempt::DefinitelyNotRun => {}
        }

        if let Some(ref iface) = adapter.interface_name {
            let wireguard_vpn = DisabledVpn {
                name: adapter.name.clone(),
                method: DisableMethod::WgQuick(iface.clone()),
            };
            let mut wg = tokio::process::Command::new("wg-quick");
            wg.args(["down", iface]);
            return run_registered_disable(wireguard_vpn, wg, TIMEOUT_MEDIUM, config, restore)
                .await;
        }
        DisableAttempt::DefinitelyNotRun
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReenableDisposition {
    IntentionallyLeftDisabled,
    Reenabled,
    IntentionallyRedisabled,
    ReconnectFailed,
    RedisableFailed,
}

impl ReenableDisposition {
    pub fn resolves_restore(self) -> bool {
        matches!(
            self,
            Self::IntentionallyLeftDisabled | Self::Reenabled | Self::IntentionallyRedisabled
        )
    }
}

/// Offer to re-enable VPNs that were disabled during the fix. The returned
/// disposition for each input VPN determines whether its restore token can be
/// resolved; failed/uncertain reconnects remain pending for the cleanup drain.
pub async fn offer_reenable(disabled: &[DisabledVpn], config: &Config) -> Vec<ReenableDisposition> {
    let mut dispositions = Vec::with_capacity(disabled.len());

    for vpn in disabled {
        let do_reenable = if is_interactive(config) {
            let prompt = format!("  Re-enable {}? (y/N): ", vpn.name);
            prompt_yes_no(&prompt)
        } else {
            false
        };

        if !do_reenable {
            dispositions.push(ReenableDisposition::IntentionallyLeftDisabled);
            continue;
        }

        let spinner = create_spinner(&format!("Re-enabling {}...", vpn.name));
        let success = reenable_vpn(vpn).await;
        spinner.finish_and_clear();

        if success {
            if is_interactive(config) {
                print_step_ok(&format!("Re-enabled {}", vpn.name), config);
            }
            // Verify connectivity after re-enable. Re-disabling the user's
            // VPN is destructive, so require two failed samples.
            let spinner = create_spinner("Verifying connectivity...");
            tokio::time::sleep(std::time::Duration::from_secs(5)).await;
            let mut connected = super::connectivity::check_connectivity().await;
            if !connected {
                spinner.set_message("Confirming connectivity loss...");
                tokio::time::sleep(std::time::Duration::from_secs(3)).await;
                connected = super::connectivity::check_connectivity().await;
            }
            spinner.finish_and_clear();

            if connected {
                dispositions.push(ReenableDisposition::Reenabled);
            } else {
                let spinner = create_spinner(&format!("Disabling {} again...", vpn.name));
                let redisabled = redisable_vpn(vpn).await;
                spinner.finish_and_clear();

                if is_interactive(config) {
                    if redisabled {
                        println!(
                            "  {} Re-enabling {} broke connectivity. The VPN has been disabled again.",
                            warn_icon(config),
                            crate::render::color::cyan(&vpn.name, config),
                        );
                    } else {
                        print_step_fail(
                            &format!("Could not confirm {} was disabled again", vpn.name),
                            "ND300 will keep reconnect cleanup registered",
                            config,
                        );
                    }
                }
                dispositions.push(if redisabled {
                    ReenableDisposition::IntentionallyRedisabled
                } else {
                    ReenableDisposition::RedisableFailed
                });
            }
        } else {
            if is_interactive(config) {
                print_step_fail(
                    &format!("Failed to re-enable {}", vpn.name),
                    "ND300 will retry during cleanup",
                    config,
                );
            }
            dispositions.push(ReenableDisposition::ReconnectFailed);
        }
    }

    dispositions
}

fn state_retry_delay() -> std::time::Duration {
    if cfg!(test) {
        std::time::Duration::from_millis(10)
    } else {
        std::time::Duration::from_secs(2)
    }
}

#[cfg(any(target_os = "macos", test))]
fn parse_scutil_state(text: &str) -> VpnConnectionState {
    match text.lines().map(str::trim).find(|line| !line.is_empty()) {
        Some(line) if line.eq_ignore_ascii_case("connected") => VpnConnectionState::Connected,
        Some(line) if line.eq_ignore_ascii_case("disconnected") => VpnConnectionState::Disconnected,
        _ => VpnConnectionState::Unknown,
    }
}

fn parse_nordvpn_state(text: &str) -> VpnConnectionState {
    for line in text.lines().map(str::trim) {
        if line.eq_ignore_ascii_case("status: connected") {
            return VpnConnectionState::Connected;
        }
        if line.eq_ignore_ascii_case("status: disconnected") {
            return VpnConnectionState::Disconnected;
        }
    }
    VpnConnectionState::Unknown
}

fn parse_expressvpn_state(text: &str) -> VpnConnectionState {
    for line in text.lines().map(str::trim) {
        let line = line.to_ascii_lowercase();
        if line == "not connected" || line == "disconnected" {
            return VpnConnectionState::Disconnected;
        }
        if line == "connected" || line.starts_with("connected to ") {
            return VpnConnectionState::Connected;
        }
    }
    VpnConnectionState::Unknown
}

fn parse_mullvad_state(text: &str) -> VpnConnectionState {
    for line in text.lines().map(str::trim) {
        let line = line.to_ascii_lowercase();
        if line == "disconnected" {
            return VpnConnectionState::Disconnected;
        }
        if line == "connected" || line.starts_with("connected to ") {
            return VpnConnectionState::Connected;
        }
    }
    VpnConnectionState::Unknown
}

fn parse_tailscale_state(text: &str) -> VpnConnectionState {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(text) else {
        return VpnConnectionState::Unknown;
    };
    let backend = value.get("BackendState").and_then(|value| value.as_str());
    let online = value
        .get("Self")
        .and_then(|value| value.get("Online"))
        .and_then(|value| value.as_bool());
    match (backend, online) {
        (Some("Running"), Some(true)) => VpnConnectionState::Connected,
        (Some("Stopped" | "NeedsLogin" | "NeedsMachineAuth"), _) | (_, Some(false)) => {
            VpnConnectionState::Disconnected
        }
        _ => VpnConnectionState::Unknown,
    }
}

fn parse_wireguard_interfaces(text: &str, expected_iface: &str) -> VpnConnectionState {
    if text
        .split_whitespace()
        .any(|interface| interface == expected_iface)
    {
        VpnConnectionState::Connected
    } else {
        // `wg show interfaces` exits successfully and lists every live
        // WireGuard interface. A successful empty/non-matching result is
        // positive evidence that this particular interface is down.
        VpnConnectionState::Disconnected
    }
}

fn parse_netsh_state(text: &str) -> VpnConnectionState {
    for line in text.lines() {
        let mut fields = line.split_whitespace();
        let (Some(admin), Some(state)) = (fields.next(), fields.next()) else {
            continue;
        };
        let admin = admin.to_ascii_lowercase();
        let state = state.to_ascii_lowercase();
        if admin == "enabled" && state == "connected" {
            return VpnConnectionState::Connected;
        }
        if matches!(admin.as_str(), "enabled" | "disabled")
            && matches!(state.as_str(), "disconnected" | "connected")
        {
            return VpnConnectionState::Disconnected;
        }
    }
    VpnConnectionState::Unknown
}

#[cfg(any(target_os = "linux", test))]
fn parse_nmcli_state(text: &str) -> VpnConnectionState {
    let state = text.trim().to_ascii_lowercase();
    if state == "activated" || state.starts_with("100 (activated)") {
        VpnConnectionState::Connected
    } else if state == "deactivated" || state.starts_with("30 (disconnected)") {
        VpnConnectionState::Disconnected
    } else {
        VpnConnectionState::Unknown
    }
}

async fn read_state_command(
    command: tokio::process::Command,
    parser: fn(&str) -> VpnConnectionState,
) -> VpnConnectionState {
    read_state_text(command)
        .await
        .map_or(VpnConnectionState::Unknown, |text| parser(&text))
}

async fn read_state_text(command: tokio::process::Command) -> Option<String> {
    let output = run_cmd(command, VPN_STATE_TIMEOUT).await.ok()?;
    if !output.status.success() {
        return None;
    }
    let mut text = String::from_utf8_lossy(&output.stdout).into_owned();
    if !output.stderr.is_empty() {
        text.push('\n');
        text.push_str(&String::from_utf8_lossy(&output.stderr));
    }
    Some(text)
}

async fn read_wireguard_state(iface: &str) -> VpnConnectionState {
    let mut command = tokio::process::Command::new("wg");
    command.args(["show", "interfaces"]);
    read_state_text(command)
        .await
        .map_or(VpnConnectionState::Unknown, |text| {
            parse_wireguard_interfaces(&text, iface)
        })
}

fn vendor_program_name(program: &str) -> String {
    std::path::Path::new(program)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or(program)
        .to_ascii_lowercase()
}

async fn read_vpn_state(vpn: &DisabledVpn) -> VpnConnectionState {
    match &vpn.method {
        DisableMethod::VendorCli(bin, reconnect_args) => {
            let program = vendor_program_name(bin);
            let (command, parser): (tokio::process::Command, fn(&str) -> VpnConnectionState) =
                match program.as_str() {
                    "nordvpn" | "nordvpn.exe" => {
                        let mut command = tokio::process::Command::new(bin);
                        command.arg("status");
                        (command, parse_nordvpn_state)
                    }
                    "expressvpn" | "expressvpn.exe" => {
                        let mut command = tokio::process::Command::new(bin);
                        command.arg("status");
                        (command, parse_expressvpn_state)
                    }
                    "mullvad" | "mullvad.exe" => {
                        let mut command = tokio::process::Command::new(bin);
                        command.arg("status");
                        (command, parse_mullvad_state)
                    }
                    "tailscale" | "tailscale.exe" => {
                        let mut command = tokio::process::Command::new(bin);
                        command.args(["status", "--json"]);
                        (command, parse_tailscale_state)
                    }
                    "wg-quick" | "wg-quick.exe" => {
                        let Some(iface) = reconnect_args.last() else {
                            return VpnConnectionState::Unknown;
                        };
                        return read_wireguard_state(iface).await;
                    }
                    _ => return VpnConnectionState::Unknown,
                };
            read_state_command(command, parser).await
        }
        DisableMethod::Netsh(iface) => {
            let mut command = tokio::process::Command::new("netsh");
            command.args(["interface", "show", "interface", &format!("name={iface}")]);
            read_state_command(command, parse_netsh_state).await
        }
        #[cfg(target_os = "macos")]
        DisableMethod::Scutil(service) => {
            let mut command = tokio::process::Command::new("/usr/sbin/scutil");
            command.args(["--nc", "status", service]);
            read_state_command(command, parse_scutil_state).await
        }
        #[cfg(target_os = "linux")]
        DisableMethod::Nmcli(conn) => {
            let mut command = tokio::process::Command::new("nmcli");
            command.args(["-g", "GENERAL.STATE", "connection", "show", conn]);
            read_state_command(command, parse_nmcli_state).await
        }
        #[cfg(target_os = "linux")]
        DisableMethod::WgQuick(iface) => read_wireguard_state(iface).await,
    }
}

async fn verify_vpn_state(vpn: &DisabledVpn, expected: VpnConnectionState) -> bool {
    for attempt in 0..VPN_STATE_ATTEMPTS {
        if read_vpn_state(vpn).await == expected {
            return true;
        }
        if attempt + 1 < VPN_STATE_ATTEMPTS {
            tokio::time::sleep(state_retry_delay()).await;
        }
    }
    false
}

async fn run_vpn_mutation(vpn: &DisabledVpn, reconnect: bool) -> bool {
    let command = match &vpn.method {
        DisableMethod::VendorCli(bin, reconnect_args) => {
            let args: Vec<String> = if reconnect {
                reconnect_args.clone()
            } else {
                reconnect_args
                    .iter()
                    .map(|arg| match arg.as_str() {
                        "connect" => "disconnect".to_string(),
                        "up" => "down".to_string(),
                        _ => arg.clone(),
                    })
                    .collect()
            };
            let mut command = tokio::process::Command::new(bin);
            command.args(args);
            command
        }
        DisableMethod::Netsh(iface) => {
            let mut command = tokio::process::Command::new("netsh");
            command.args([
                "interface",
                "set",
                "interface",
                iface,
                if reconnect { "enabled" } else { "disabled" },
            ]);
            command
        }
        #[cfg(target_os = "macos")]
        DisableMethod::Scutil(service) => {
            let mut command = tokio::process::Command::new("/usr/sbin/scutil");
            command.args(["--nc", if reconnect { "start" } else { "stop" }, service]);
            command
        }
        #[cfg(target_os = "linux")]
        DisableMethod::Nmcli(conn) => {
            let mut command = tokio::process::Command::new("nmcli");
            command.args(["connection", if reconnect { "up" } else { "down" }, conn]);
            command
        }
        #[cfg(target_os = "linux")]
        DisableMethod::WgQuick(iface) => {
            let mut command = tokio::process::Command::new("wg-quick");
            command.args([if reconnect { "up" } else { "down" }, iface]);
            command
        }
    };

    matches!(
        run_owned_mutation(command, VPN_MUTATION_TIMEOUT).await,
        Ok(output) if output.status.success()
    )
}

pub(super) async fn reenable_vpn(vpn: &DisabledVpn) -> bool {
    run_vpn_mutation(vpn, true).await && verify_vpn_state(vpn, VpnConnectionState::Connected).await
}

pub(super) async fn redisable_vpn(vpn: &DisabledVpn) -> bool {
    run_vpn_mutation(vpn, false).await
        && verify_vpn_state(vpn, VpnConnectionState::Disconnected).await
}

/// Serialize VPN state for JSON output.
pub fn vpn_json(disabled: &[DisabledVpn]) -> serde_json::Value {
    if disabled.is_empty() {
        return serde_json::json!(null);
    }
    let items: Vec<serde_json::Value> = disabled
        .iter()
        .map(|v| {
            serde_json::json!({
                "name": v.name,
                "disabled": true,
            })
        })
        .collect();
    serde_json::json!(items)
}

#[cfg(test)]
mod reconnect_verification_tests {
    use super::*;

    #[test]
    fn scutil_requires_an_exact_connected_status_line() {
        assert_eq!(
            parse_scutil_state("Connected\nExtended Status <dictionary> {}"),
            VpnConnectionState::Connected
        );
        assert_eq!(
            parse_scutil_state("Disconnected\nExtended Status <dictionary> {}"),
            VpnConnectionState::Disconnected
        );
        assert_eq!(
            parse_scutil_state("Connecting\nExtended Status <dictionary> {}"),
            VpnConnectionState::Unknown
        );
    }

    #[test]
    fn consumer_provider_parsers_do_not_confuse_disconnected_with_connected() {
        assert_eq!(
            parse_nordvpn_state("Status: Connected\nHostname: us1234"),
            VpnConnectionState::Connected
        );
        assert_eq!(
            parse_nordvpn_state("Status: Disconnected"),
            VpnConnectionState::Disconnected
        );
        assert_eq!(
            parse_expressvpn_state("Connected to USA - New York"),
            VpnConnectionState::Connected
        );
        assert_eq!(
            parse_expressvpn_state("Not connected"),
            VpnConnectionState::Disconnected
        );
        assert_eq!(
            parse_mullvad_state("Disconnected"),
            VpnConnectionState::Disconnected
        );
    }

    #[test]
    fn tailscale_requires_running_and_online() {
        assert_eq!(
            parse_tailscale_state(r#"{"BackendState":"Running","Self":{"Online":true}}"#),
            VpnConnectionState::Connected
        );
        assert_eq!(
            parse_tailscale_state(r#"{"BackendState":"Running","Self":{"Online":false}}"#),
            VpnConnectionState::Disconnected
        );
        assert_eq!(
            parse_tailscale_state(r#"{"BackendState":"Running","Self":{}}"#),
            VpnConnectionState::Unknown
        );
    }

    #[test]
    fn platform_state_parsers_are_fail_closed() {
        let netsh = "Admin State    State          Type             Interface Name\n\
                     Enabled        Connected      Dedicated        Test VPN";
        assert_eq!(parse_netsh_state(netsh), VpnConnectionState::Connected);
        assert_eq!(
            parse_netsh_state(&netsh.replace("Connected", "Disconnected")),
            VpnConnectionState::Disconnected
        );
        assert_eq!(
            parse_nmcli_state("100 (activated)"),
            VpnConnectionState::Connected
        );
        assert_eq!(
            parse_nmcli_state("30 (disconnected)"),
            VpnConnectionState::Disconnected
        );
        assert_eq!(
            parse_nmcli_state("0 (unknown)"),
            VpnConnectionState::Unknown
        );
        assert_eq!(
            parse_wireguard_interfaces("wg0 wg-office", "wg0"),
            VpnConnectionState::Connected
        );
        assert_eq!(
            parse_wireguard_interfaces("wg-office", "wg0"),
            VpnConnectionState::Disconnected
        );
    }

    #[tokio::test]
    async fn exit_zero_without_supported_state_readback_never_confirms_reconnect() {
        let vpn = DisabledVpn {
            name: "Unknown test provider".to_string(),
            method: DisableMethod::VendorCli("rustc".to_string(), vec!["--version".to_string()]),
        };

        assert!(!reenable_vpn(&vpn).await);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn supported_provider_reconnect_requires_and_accepts_positive_readback() {
        use std::os::unix::fs::PermissionsExt;
        use std::sync::atomic::{AtomicU64, Ordering};

        static NEXT_ID: AtomicU64 = AtomicU64::new(0);
        let root = std::env::temp_dir().join(format!(
            "nd300-vpn-readback-{}-{}",
            std::process::id(),
            NEXT_ID.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&root).expect("create fake provider directory");
        let provider = root.join("nordvpn");
        std::fs::write(
            &provider,
            "#!/bin/sh\nif [ \"$1\" = status ]; then printf 'Status: Connected\\n'; fi\n",
        )
        .expect("write fake provider");
        std::fs::set_permissions(&provider, std::fs::Permissions::from_mode(0o700))
            .expect("make fake provider executable");

        let vpn = DisabledVpn {
            name: "Fake NordVPN".to_string(),
            method: DisableMethod::VendorCli(
                provider.to_string_lossy().into_owned(),
                vec!["connect".to_string()],
            ),
        };

        assert!(reenable_vpn(&vpn).await);
        std::fs::remove_dir_all(root).expect("remove fake provider directory");
    }
}

#[cfg(test)]
mod restore_registration_tests {
    use super::*;
    use crate::actions::fix::session::RestoreRegistry;

    fn test_vpn() -> DisabledVpn {
        DisabledVpn {
            name: "Test VPN".to_string(),
            method: DisableMethod::VendorCli("rustc".to_string(), vec!["--version".to_string()]),
        }
    }

    #[tokio::test]
    async fn spawned_nonzero_disconnect_keeps_reconnect_pending() {
        let restore = RestoreRegistry::new();
        let mut command = tokio::process::Command::new("rustc");
        command.arg("--definitely-not-a-rustc-option");

        let result = run_registered_disable(
            test_vpn(),
            command,
            TIMEOUT_MEDIUM,
            &Config::new().with_json(),
            &restore,
        )
        .await;

        assert!(matches!(result, DisableAttempt::Uncertain));
        assert_eq!(restore.pending_count().await, 1);
    }

    #[tokio::test]
    async fn spawn_failure_is_the_only_failure_that_resolves_reconnect() {
        let restore = RestoreRegistry::new();
        let command = tokio::process::Command::new("nd300-nonexistent-vpn-disconnector");

        let result = run_registered_disable(
            test_vpn(),
            command,
            TIMEOUT_MEDIUM,
            &Config::new().with_json(),
            &restore,
        )
        .await;

        assert!(matches!(result, DisableAttempt::DefinitelyNotRun));
        assert_eq!(restore.pending_count().await, 0);
    }

    #[tokio::test]
    async fn exit_zero_with_unknown_reconnect_state_stays_pending_after_drain() {
        use crate::actions::fix::session::RestoreOp;
        use std::sync::Arc;

        let restore = RestoreRegistry::new();
        let vpn = DisabledVpn {
            name: "Unknown test provider".to_string(),
            method: DisableMethod::VendorCli("rustc".to_string(), vec!["--version".to_string()]),
        };
        restore
            .register(RestoreOp::ReEnableVpn(Arc::new(vpn)))
            .await;

        let failures = restore.drain().await;

        assert_eq!(failures.len(), 1, "failures: {failures:?}");
        assert_eq!(restore.pending_count().await, 1);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn timed_out_disconnect_is_killed_reaped_and_keeps_reconnect_pending() {
        let restore = RestoreRegistry::new();
        let mut command = tokio::process::Command::new("sh");
        command.args(["-c", "sleep 30"]);

        let result = run_registered_disable(
            test_vpn(),
            command,
            std::time::Duration::from_millis(1),
            &Config::new().with_json(),
            &restore,
        )
        .await;

        assert!(matches!(result, DisableAttempt::Uncertain));
        assert_eq!(restore.pending_count().await, 1);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn cancelled_disconnect_is_terminated_before_restore_cleanup_can_race_it() {
        let root =
            std::env::temp_dir().join(format!("nd300-vpn-cancel-test-{}", std::process::id()));
        let started = root.with_extension("started");
        let late = root.with_extension("late");
        let _ = std::fs::remove_file(&started);
        let _ = std::fs::remove_file(&late);

        let restore = RestoreRegistry::new();
        let task_restore = restore.clone();
        let started_arg = started.to_string_lossy().into_owned();
        let late_arg = late.to_string_lossy().into_owned();
        let task = tokio::spawn(async move {
            let mut command = tokio::process::Command::new("sh");
            command.args([
                "-c",
                "printf started > \"$1\"; sleep 1; printf late > \"$2\"",
                "nd300-vpn-cancel-test",
                &started_arg,
                &late_arg,
            ]);
            run_registered_disable(
                test_vpn(),
                command,
                TIMEOUT_MEDIUM,
                &Config::new().with_json(),
                &task_restore,
            )
            .await
        });

        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(2);
        while !started.exists() && tokio::time::Instant::now() < deadline {
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        assert!(
            started.exists(),
            "disconnect child never reached its test body"
        );
        task.abort();
        let _ = task.await;

        // Start cleanup immediately. The reconnect mutation uses the same
        // serialized owner, so it cannot overtake the cancelled disconnect's
        // kill-and-reap supervisor.
        let failures = restore.drain().await;
        assert_eq!(failures.len(), 1, "failures: {failures:?}");
        tokio::time::sleep(std::time::Duration::from_millis(1200)).await;

        assert!(
            !late.exists(),
            "cancelled disconnect child survived long enough to mutate state after cleanup"
        );
        assert_eq!(restore.pending_count().await, 1);
        let _ = std::fs::remove_file(started);
        let _ = std::fs::remove_file(late);
    }

    #[test]
    fn only_confirmed_or_intentional_reenable_dispositions_resolve_restore() {
        assert!(ReenableDisposition::IntentionallyLeftDisabled.resolves_restore());
        assert!(ReenableDisposition::Reenabled.resolves_restore());
        assert!(ReenableDisposition::IntentionallyRedisabled.resolves_restore());
        assert!(!ReenableDisposition::ReconnectFailed.resolves_restore());
        assert!(!ReenableDisposition::RedisableFailed.resolves_restore());
    }
}
