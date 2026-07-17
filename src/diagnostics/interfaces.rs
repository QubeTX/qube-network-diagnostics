use serde::Serialize;
use std::collections::HashMap;
#[cfg(any(target_os = "macos", test))]
use std::collections::HashSet;
use sysinfo::Networks;

use super::DiagnosticResult;

#[derive(Debug, Clone, Serialize)]
pub struct InterfaceInfo {
    pub name: String,
    pub mac: String,
    pub ip_addresses: Vec<String>,
    pub is_up: bool,
    pub interface_type: String,
    pub rx_bytes: u64,
    pub tx_bytes: u64,
}

/// Owned, Send-safe per-interface fields extracted from `sysinfo::Networks`
/// inside the blocking closure, so the (blocking) enumeration runs off the
/// async runtime while the data crosses back to the async loop below.
struct RawInterface {
    name: String,
    mac_bytes: [u8; 6],
    ip_addrs: Vec<String>,
    rx_bytes: u64,
    tx_bytes: u64,
}

/// Names which identify the same OS interface. On Windows, `default-net`
/// exposes the opaque IP Helper `AdapterName` (normally a GUID) as `name`,
/// while `sysinfo` keys its network rows by the human-readable FriendlyName.
/// Keep all aliases so the default route is matched to the correct detail row.
#[derive(Default)]
struct InterfaceAliases {
    values: Vec<String>,
}

impl InterfaceAliases {
    fn from_default_interface(interface: default_net::Interface) -> Self {
        let mut values = vec![interface.name];
        values.extend(interface.friendly_name);
        values.extend(interface.description);
        Self { values }
    }

    fn is_empty(&self) -> bool {
        self.values.is_empty()
    }

    fn matches(&self, name: &str) -> bool {
        self.values
            .iter()
            .any(|alias| interface_aliases_match(alias, name))
    }

    fn identifies_type(&self, interface_type: &str) -> bool {
        self.values
            .iter()
            .any(|alias| detect_interface_type(alias) == interface_type)
    }
}

pub async fn check() -> (DiagnosticResult, Vec<InterfaceInfo>) {
    // `Networks::new_with_refreshed_list` is a synchronous, blocking system
    // enumeration — the heaviest sync call in the core diagnostics. Run it off
    // the async runtime and return owned data. A JoinError falls back to an
    // empty list (same as no interfaces).
    let (raw_interfaces, default_interface, operational_states): (
        Vec<RawInterface>,
        InterfaceAliases,
        HashMap<String, bool>,
    ) = tokio::task::spawn_blocking(|| {
        let networks = Networks::new_with_refreshed_list();
        let interfaces = networks
            .iter()
            .map(|(name, data)| RawInterface {
                name: name.clone(),
                mac_bytes: data.mac_address().0,
                ip_addrs: data
                    .ip_networks()
                    .iter()
                    .map(|n| n.addr.to_string())
                    .collect(),
                rx_bytes: data.total_received(),
                tx_bytes: data.total_transmitted(),
            })
            .collect();
        let os_interfaces = default_net::get_interfaces();
        let default_interface = default_net::get_default_interface()
            .ok()
            .map(InterfaceAliases::from_default_interface)
            .unwrap_or_default();
        let mut operational_states = HashMap::new();
        for interface in os_interfaces {
            // `default-net` reports native interface flags on Unix. Linux's
            // IFF_UP is only administrative state, so require IFF_RUNNING as
            // carrier evidence. Windows synthesizes IFF_UP from OperStatus.
            let is_up = operational_from_flags(interface.flags, cfg!(target_os = "linux"));
            insert_operational_alias(&mut operational_states, &interface.name, is_up);
            if let Some(name) = interface.friendly_name {
                insert_operational_alias(&mut operational_states, &name, is_up);
            }
            if let Some(description) = interface.description {
                insert_operational_alias(&mut operational_states, &description, is_up);
            }
        }
        (interfaces, default_interface, operational_states)
    })
    .await
    .unwrap_or_default();

    #[cfg(target_os = "macos")]
    let macos_topology = collect_macos_topology().await;

    let mut details = Vec::new();
    let mut active_count = 0;
    let mut wifi_info = String::new();

    for raw in raw_interfaces {
        let mac = format_mac(raw.mac_bytes);
        let ip_addrs = raw.ip_addrs;
        // Traffic counters describe history, not current link state. A quiet
        // but configured link is still up; conversely a dormant tunnel may
        // retain counters long after it stopped carrying traffic.
        #[cfg(target_os = "macos")]
        let is_up = macos_topology
            .operational
            .get(&raw.name)
            .copied()
            .or_else(|| {
                operational_states
                    .get(&normalize_interface_alias(&raw.name))
                    .copied()
            })
            .unwrap_or_else(|| has_usable_address(&ip_addrs));
        #[cfg(target_os = "macos")]
        let is_up = if raw.name.to_ascii_lowercase().starts_with("utun") {
            // Keep the fail-closed tunnel rule even if ifconfig parsing was
            // unavailable and the fallback flag source only reported IFF_UP.
            is_up && macos_topology.active_interfaces.contains(&raw.name)
        } else {
            is_up
        };
        #[cfg(not(target_os = "macos"))]
        let is_up = operational_states
            .get(&normalize_interface_alias(&raw.name))
            .copied()
            .unwrap_or_else(|| fallback_operational_state(&ip_addrs));

        #[cfg(target_os = "macos")]
        let iface_type = macos_topology
            .hardware_ports
            .get(&raw.name)
            .map(|port| classify_hardware_port(port))
            .unwrap_or_else(|| detect_interface_type(&raw.name));
        #[cfg(not(target_os = "macos"))]
        let iface_type = detect_interface_type(&raw.name);

        let is_default = default_interface.matches(&raw.name);
        #[cfg(target_os = "macos")]
        let hardware_mapped = macos_topology.hardware_ports.contains_key(&raw.name);
        #[cfg(not(target_os = "macos"))]
        let hardware_mapped = false;
        if is_up
            && has_non_link_local_address(&ip_addrs)
            && is_user_uplink(&raw.name, &iface_type, is_default, hardware_mapped)
        {
            active_count += 1;
            if iface_type == "Wi-Fi"
                && wifi_info.is_empty()
                && (is_default || default_interface.is_empty())
            {
                // Core mode must stay fast and must not invoke the deprecated
                // private `airport` tool. Rich radio data belongs to the deep
                // system_profiler-backed Wi-Fi diagnostic.
                #[cfg(target_os = "macos")]
                {
                    wifi_info = "Wi-Fi".to_string();
                }
                #[cfg(not(target_os = "macos"))]
                {
                    wifi_info = get_wifi_summary().await;
                }
            }
        }

        details.push(InterfaceInfo {
            name: raw.name,
            mac,
            ip_addresses: ip_addrs,
            is_up,
            interface_type: iface_type,
            rx_bytes: raw.rx_bytes,
            tx_bytes: raw.tx_bytes,
        });
    }

    let default_active = details
        .iter()
        .find(|interface| interface.is_up && default_interface.matches(&interface.name));
    let underlying_types: Vec<&str> = details
        .iter()
        .filter(|interface| {
            interface.is_up
                && has_non_link_local_address(&interface.ip_addresses)
                && matches!(
                    interface.interface_type.as_str(),
                    "Wi-Fi" | "Ethernet" | "Bluetooth"
                )
        })
        .map(|interface| interface.interface_type.as_str())
        .collect();
    let default_is_tunnel = default_active
        .is_some_and(|interface| interface.interface_type == "VPN/Tunnel")
        || default_interface.identifies_type("VPN/Tunnel");
    let result = if default_is_tunnel {
        let description = describe_tunnel_over(&underlying_types);
        DiagnosticResult::ok("Network", format!("Connected via {description}"))
    } else if active_count == 0 {
        DiagnosticResult::fail("Network", "No active network interfaces found")
    } else if let Some(interface) = default_active {
        let description = if interface.interface_type == "Wi-Fi" && !wifi_info.is_empty() {
            wifi_info.clone()
        } else {
            interface.interface_type.clone()
        };
        DiagnosticResult::ok("Network", format!("Connected via {description}"))
    } else if !wifi_info.is_empty() {
        DiagnosticResult::ok("Network", format!("Connected via {}", wifi_info))
    } else if active_count == 1 {
        let active = default_active.or_else(|| details.iter().find(|i| i.is_up));
        let desc = match active {
            Some(iface) => format!("Connected via {}", iface.interface_type),
            None => "Connected".to_string(),
        };
        DiagnosticResult::ok("Network", desc)
    } else {
        DiagnosticResult::ok("Network", format!("{} active interfaces", active_count))
    };

    (result, details)
}

fn detect_interface_type(name: &str) -> String {
    let lower = name.to_lowercase();
    if lower == "lo" || lower == "lo0" || (lower.starts_with("lo") && lower.len() <= 3) {
        "Loopback".to_string()
    } else if lower.contains("tailscale")
        || lower.contains("wintun")
        || lower.contains("wireguard")
        || lower.contains("nordlynx")
        || lower.contains("anyconnect")
        || lower.contains("cisco secure")
        || lower.contains("globalprotect")
        || lower.contains("pangp")
        || lower.contains("palo alto")
        || lower.contains("fortinet")
        || lower.contains("forticlient")
        || lower.contains("zscaler")
        || lower.contains("check point")
        || lower.contains("checkpoint")
        || lower.contains("juniper")
        || lower.contains("zerotier")
        || lower.contains("hamachi")
        || lower.contains("openvpn")
        || lower.contains("vpn")
        || lower.contains("utun")
        || lower.contains("tap-windows")
        || lower.starts_with("tap")
        || lower.starts_with("tun")
        || lower.starts_with("wg")
        || lower.starts_with("ppp")
    {
        "VPN/Tunnel".to_string()
    } else if lower.contains("vethernet")
        || lower.contains("hyper-v")
        || lower.contains("hyperv")
        || lower.contains("default switch")
        || lower.contains("vmware")
        || lower.contains("vmnet")
        || lower.contains("virtualbox")
        || lower.contains("vbox")
        || lower.contains("wsl")
        || lower.contains("docker")
        || lower.contains("veth")
        || lower.starts_with("br-")
    {
        // Check virtual adapters before the generic Ethernet token: Windows
        // exposes Hyper-V/WSL links as "vEthernet (...)".
        "Virtual".to_string()
    } else if lower.contains("wi-fi")
        || lower.contains("wifi")
        || lower.contains("wireless")
        || lower.contains("wlan")
        || lower.contains("wlp")
    {
        "Wi-Fi".to_string()
    } else if lower.contains("eth")
        || lower.contains("enp")
        || lower.contains("eno")
        || lower.contains("ethernet")
        || lower.contains("gbe")
        || lower.contains("realtek")
    {
        "Ethernet".to_string()
    } else if lower.contains("bluetooth") || lower.contains("bnep") {
        "Bluetooth".to_string()
    } else {
        "Unknown".to_string()
    }
}

#[cfg(any(target_os = "macos", target_os = "windows"))]
fn has_usable_address(addrs: &[String]) -> bool {
    addrs.iter().any(|addr| {
        let bare = addr.split('%').next().unwrap_or(addr);
        bare.parse::<std::net::IpAddr>()
            .map(|ip| !ip.is_unspecified())
            .unwrap_or(false)
    })
}

fn normalize_interface_alias(name: &str) -> String {
    let normalized = name.trim().to_ascii_lowercase();
    let normalized = normalized
        .strip_prefix(r"\device\tcpip_")
        .unwrap_or(&normalized);
    normalized
        .strip_prefix('{')
        .and_then(|value| value.strip_suffix('}'))
        .unwrap_or(normalized)
        .to_string()
}

fn interface_aliases_match(left: &str, right: &str) -> bool {
    normalize_interface_alias(left) == normalize_interface_alias(right)
}

fn insert_operational_alias(states: &mut HashMap<String, bool>, name: &str, is_up: bool) {
    states.insert(normalize_interface_alias(name), is_up);
}

fn operational_from_flags(flags: u32, require_running: bool) -> bool {
    const IFF_UP: u32 = 0x1;
    const IFF_RUNNING: u32 = 0x40;
    flags & IFF_UP != 0 && (!require_running || flags & IFF_RUNNING != 0)
}

#[cfg(target_os = "linux")]
fn fallback_operational_state(_addrs: &[String]) -> bool {
    // An assigned address does not prove carrier. If OS flag enumeration did
    // not produce this Linux interface, fail closed rather than turning an
    // administratively-up but unplugged link into an operational one.
    false
}

#[cfg(target_os = "windows")]
fn fallback_operational_state(addrs: &[String]) -> bool {
    has_usable_address(addrs)
}

fn describe_tunnel_over(underlying_types: &[&str]) -> String {
    let mut types = underlying_types.to_vec();
    types.sort_unstable();
    types.dedup();
    match types.as_slice() {
        [] => "VPN/Tunnel".to_string(),
        [interface_type] => format!("VPN/Tunnel over {interface_type}"),
        _ => format!(
            "VPN/Tunnel over ambiguous active uplinks ({})",
            types.join(", ")
        ),
    }
}

fn has_non_link_local_address(addrs: &[String]) -> bool {
    addrs.iter().any(|address| {
        let bare = address.split('%').next().unwrap_or(address);
        match bare.parse::<std::net::IpAddr>() {
            Ok(std::net::IpAddr::V4(ip)) => {
                !ip.is_unspecified() && !ip.is_loopback() && !ip.is_link_local()
            }
            Ok(std::net::IpAddr::V6(ip)) => {
                !ip.is_unspecified() && !ip.is_loopback() && !ip.is_unicast_link_local()
            }
            Err(_) => false,
        }
    })
}

fn is_user_uplink(
    name: &str,
    interface_type: &str,
    is_default: bool,
    hardware_mapped: bool,
) -> bool {
    let lower = name.to_ascii_lowercase();
    if lower == "bridge0" {
        // A Thunderbolt Bridge is a real hardware-mapped uplink on macOS even
        // though its BSD device name resembles a virtual bridge. Count it only
        // when it actually owns the default route; ordinary software bridges
        // remain excluded.
        return hardware_mapped && is_default && interface_type == "Ethernet";
    }
    if matches!(lower.as_str(), "lo" | "lo0" | "awdl0" | "llw0" | "ap1")
        || lower.starts_with("anpi")
        || lower.starts_with("utun")
        || lower.starts_with("gif")
        || lower.starts_with("stf")
    {
        return false;
    }
    matches!(interface_type, "Wi-Fi" | "Ethernet" | "Bluetooth")
        || (is_default
            && interface_type != "Loopback"
            && interface_type != "Virtual"
            && interface_type != "VPN/Tunnel")
}

#[cfg(any(target_os = "macos", test))]
fn classify_hardware_port(port: &str) -> String {
    let lower = port.to_ascii_lowercase();
    if lower.contains("wi-fi") || lower.contains("airport") {
        "Wi-Fi".to_string()
    } else if lower.contains("ethernet") || lower.contains("thunderbolt") {
        "Ethernet".to_string()
    } else if lower.contains("bluetooth") {
        "Bluetooth".to_string()
    } else {
        "Unknown".to_string()
    }
}

#[cfg(target_os = "macos")]
#[derive(Default)]
struct MacosTopology {
    hardware_ports: HashMap<String, String>,
    operational: HashMap<String, bool>,
    active_interfaces: HashSet<String>,
}

#[cfg(target_os = "macos")]
async fn collect_macos_topology() -> MacosTopology {
    let mut hardware_cmd = tokio::process::Command::new("/usr/sbin/networksetup");
    hardware_cmd.arg("-listallhardwareports");
    let ifconfig_cmd = tokio::process::Command::new("/sbin/ifconfig");
    let mut nwi_cmd = tokio::process::Command::new("/usr/sbin/scutil");
    nwi_cmd.arg("--nwi");
    let mut routes4_cmd = tokio::process::Command::new("/usr/sbin/netstat");
    routes4_cmd.args(["-rn", "-f", "inet"]);
    let mut routes6_cmd = tokio::process::Command::new("/usr/sbin/netstat");
    routes6_cmd.args(["-rn", "-f", "inet6"]);
    let (hardware, ifconfig, nwi, routes4, routes6) = tokio::join!(
        super::util::run_with_timeout(hardware_cmd, super::util::QUICK),
        super::util::run_with_timeout(ifconfig_cmd, super::util::QUICK),
        super::util::run_with_timeout(nwi_cmd, super::util::QUICK),
        super::util::run_with_timeout(routes4_cmd, super::util::QUICK),
        super::util::run_with_timeout(routes6_cmd, super::util::QUICK),
    );

    let mut active_interfaces = nwi
        .map(|out| parse_macos_nwi_interfaces(&String::from_utf8_lossy(&out.stdout)))
        .unwrap_or_default();
    for output in [routes4, routes6].into_iter().flatten() {
        active_interfaces.extend(parse_macos_routed_interfaces(&String::from_utf8_lossy(
            &output.stdout,
        )));
    }

    MacosTopology {
        hardware_ports: hardware
            .map(|out| parse_macos_hardware_ports(&String::from_utf8_lossy(&out.stdout)))
            .unwrap_or_default(),
        operational: ifconfig
            .map(|out| {
                parse_macos_operational(&String::from_utf8_lossy(&out.stdout), &active_interfaces)
            })
            .unwrap_or_default(),
        active_interfaces,
    }
}

#[cfg(any(target_os = "macos", test))]
fn parse_macos_hardware_ports(text: &str) -> HashMap<String, String> {
    let mut ports = HashMap::new();
    let mut hardware_port: Option<String> = None;
    for line in text.lines().map(str::trim) {
        if let Some(port) = line.strip_prefix("Hardware Port:") {
            hardware_port = Some(port.trim().to_string());
        } else if let Some(device) = line.strip_prefix("Device:") {
            if let Some(port) = hardware_port.take() {
                ports.insert(device.trim().to_string(), port);
            }
        } else if line.is_empty() {
            hardware_port = None;
        }
    }
    ports
}

#[cfg(any(target_os = "macos", test))]
fn parse_macos_nwi_interfaces(text: &str) -> HashSet<String> {
    let mut active = HashSet::new();
    let mut current: Option<&str> = None;
    for line in text.lines().map(str::trim) {
        if let Some(interfaces) = line.strip_prefix("Network interfaces:") {
            active.extend(interfaces.split_whitespace().map(str::to_string));
        } else if line.contains(" : flags") || line.contains(": flags") {
            current = line.split_once(':').map(|(name, _)| name.trim());
        } else if line.starts_with("reach") && line.contains("Reachable") {
            if let Some(name) = current {
                active.insert(name.to_string());
            }
        }
    }
    active
}

#[cfg(any(target_os = "macos", test))]
fn parse_macos_routed_interfaces(text: &str) -> HashSet<String> {
    text.lines()
        .filter_map(|line| {
            let fields: Vec<&str> = line.split_whitespace().collect();
            (fields.len() >= 4
                && !matches!(
                    fields[0],
                    "Destination" | "Routing" | "Internet:" | "Internet6:"
                )
                && meaningful_macos_route(fields[0], fields[1]))
            .then(|| fields[3].to_string())
        })
        .collect()
}

#[cfg(any(target_os = "macos", test))]
fn meaningful_macos_route(destination: &str, gateway: &str) -> bool {
    let destination = destination.to_ascii_lowercase();
    let gateway = gateway.to_ascii_lowercase();

    if destination == "default" {
        // macOS installs interface-scoped IPv6 defaults on dormant system
        // tunnels. A link-local gateway alone is not positive tunnel evidence.
        return !gateway.starts_with("fe80::");
    }

    !(destination.starts_with("fe80")
        || destination.starts_with("ff")
        || destination.starts_with("169.254")
        || destination.starts_with("::1")
        || destination.starts_with("link#"))
}

#[cfg(any(target_os = "macos", test))]
fn parse_macos_operational(
    text: &str,
    active_interfaces: &HashSet<String>,
) -> HashMap<String, bool> {
    let mut states = HashMap::new();
    let mut current: Option<String> = None;
    let mut flags_up = false;
    let mut explicit_status: Option<bool> = None;
    let mut has_address = false;

    let flush = |states: &mut HashMap<String, bool>,
                 current: &mut Option<String>,
                 flags_up: bool,
                 explicit_status: Option<bool>,
                 has_address: bool,
                 active_interfaces: &HashSet<String>| {
        if let Some(name) = current.take() {
            let mut operational = flags_up && explicit_status.unwrap_or(has_address);
            if name.to_ascii_lowercase().starts_with("utun") {
                // Darwin retains dormant utun devices with UP/RUNNING and a
                // link-local address. Require routing or NetworkInformation
                // reachability evidence before calling one operational.
                operational &= active_interfaces.contains(&name);
            }
            states.insert(name, operational);
        }
    };

    for line in text.lines() {
        if !line.starts_with(char::is_whitespace) && line.contains(": flags=") {
            flush(
                &mut states,
                &mut current,
                flags_up,
                explicit_status,
                has_address,
                active_interfaces,
            );
            current = line.split_once(':').map(|(name, _)| name.to_string());
            flags_up = line
                .split_once('<')
                .and_then(|(_, rest)| rest.split_once('>'))
                .is_some_and(|(flags, _)| flags.split(',').any(|flag| flag == "UP"));
            explicit_status = None;
            has_address = false;
        } else {
            let trimmed = line.trim();
            if let Some(status) = trimmed.strip_prefix("status:") {
                explicit_status = Some(status.trim().eq_ignore_ascii_case("active"));
            } else if trimmed.starts_with("inet ") || trimmed.starts_with("inet6 ") {
                has_address = true;
            }
        }
    }
    flush(
        &mut states,
        &mut current,
        flags_up,
        explicit_status,
        has_address,
        active_interfaces,
    );
    states
}

fn format_mac(bytes: [u8; 6]) -> String {
    if bytes == [0, 0, 0, 0, 0, 0] {
        return "N/A".to_string();
    }
    format!(
        "{:02X}:{:02X}:{:02X}:{:02X}:{:02X}:{:02X}",
        bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5]
    )
}

#[cfg(not(target_os = "macos"))]
async fn get_wifi_summary() -> String {
    #[cfg(windows)]
    {
        let mut cmd = tokio::process::Command::new("netsh");
        cmd.args(["wlan", "show", "interfaces"]);
        match super::util::run_with_timeout(cmd, super::util::QUICK).await {
            Some(output) => {
                let text = String::from_utf8_lossy(&output.stdout);
                let mut band = String::new();
                let mut ssid = String::new();

                for line in text.lines() {
                    let line = line.trim();
                    if line.starts_with("SSID")
                        && !line.starts_with("SSID B")
                        && !line.starts_with("SSID name")
                    {
                        if let Some(val) = line.split(':').nth(1) {
                            ssid = val.trim().to_string();
                        }
                    }
                    if line.starts_with("Radio type") || line.starts_with("Band") {
                        if let Some(val) = line.split(':').nth(1) {
                            let val = val.trim();
                            if val.contains("6 GHz") || val.contains("6E") {
                                band = "6 GHz".to_string();
                            } else if val.contains("5 GHz")
                                || val.contains("802.11a")
                                || val.contains("802.11ac")
                            {
                                band = "5 GHz".to_string();
                            } else if val.contains("802.11ax") {
                                // 802.11ax can be 2.4/5/6 GHz; leave band empty to rely on channel
                                band = String::new();
                            } else {
                                band = "2.4 GHz".to_string();
                            }
                        }
                    }
                    // Also check "Channel" for band detection fallback
                    if line.starts_with("Channel") {
                        if let Some(val) = line.split(':').nth(1) {
                            if let Ok(ch) = val.trim().parse::<u32>() {
                                if band.is_empty() {
                                    if ch > 14 && ch <= 177 {
                                        band = "5 GHz".to_string();
                                    } else if ch <= 14 {
                                        band = "2.4 GHz".to_string();
                                    }
                                }
                            }
                        }
                    }
                }

                if !ssid.is_empty() {
                    if !band.is_empty() {
                        format!("Wi-Fi ({}) - {}", band, ssid)
                    } else {
                        format!("Wi-Fi - {}", ssid)
                    }
                } else {
                    "Wi-Fi".to_string()
                }
            }
            None => "Wi-Fi".to_string(),
        }
    }

    #[cfg(target_os = "linux")]
    {
        let mut cmd = tokio::process::Command::new("iwgetid");
        cmd.args(["-r"]);
        match super::util::run_with_timeout(cmd, super::util::QUICK).await {
            Some(output) => {
                let ssid = String::from_utf8_lossy(&output.stdout).trim().to_string();
                // Try to get frequency
                let mut freq_cmd = tokio::process::Command::new("iwgetid");
                freq_cmd.args(["--freq", "-r"]);
                let band = match super::util::run_with_timeout(freq_cmd, super::util::QUICK).await {
                    Some(freq_out) => {
                        let freq_str = String::from_utf8_lossy(&freq_out.stdout).trim().to_string();
                        if let Ok(freq) = freq_str.parse::<f64>() {
                            if freq > 5.0 {
                                "5 GHz".to_string()
                            } else if freq > 2.0 {
                                "2.4 GHz".to_string()
                            } else {
                                String::new()
                            }
                        } else {
                            String::new()
                        }
                    }
                    None => String::new(),
                };

                if !ssid.is_empty() {
                    if !band.is_empty() {
                        format!("Wi-Fi ({}) - {}", band, ssid)
                    } else {
                        format!("Wi-Fi - {}", ssid)
                    }
                } else {
                    "Wi-Fi".to_string()
                }
            }
            None => "Wi-Fi".to_string(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn macos_hardware_ports_identify_en0_wifi() {
        let fixture = "Hardware Port: Ethernet Adapter (en3)\nDevice: en3\n\nHardware Port: Wi-Fi\nDevice: en0\n\nVLAN Configurations\n";
        let ports = parse_macos_hardware_ports(fixture);
        assert_eq!(ports.get("en0").map(String::as_str), Some("Wi-Fi"));
        assert_eq!(classify_hardware_port(&ports["en0"]), "Wi-Fi");
        assert_eq!(classify_hardware_port(&ports["en3"]), "Ethernet");
    }

    #[test]
    fn macos_operational_uses_status_not_historical_traffic() {
        let fixture = "en0: flags=8863<UP,BROADCAST,RUNNING> mtu 1500\n\tinet 10.1.0.91 netmask 0xffffff00\n\tstatus: active\nen3: flags=8863<UP,BROADCAST,RUNNING> mtu 1500\n\tstatus: inactive\nutun0: flags=8051<UP,POINTOPOINT,RUNNING> mtu 1380\n\tinet6 fe80::1%utun0 prefixlen 64\nutun7: flags=8051<UP,POINTOPOINT,RUNNING> mtu 1380\n\tinet6 fe80::2%utun7 prefixlen 64\n";
        let active = ["utun7".to_string()].into_iter().collect();
        let states = parse_macos_operational(fixture, &active);
        assert_eq!(states.get("en0"), Some(&true));
        assert_eq!(states.get("en3"), Some(&false));
        assert_eq!(states.get("utun0"), Some(&false));
        assert_eq!(states.get("utun7"), Some(&true));
    }

    #[test]
    fn macos_nwi_and_routes_exclude_dormant_link_local_tunnels() {
        let nwi = "IPv4 network interface information\n     en0 : flags : 0x5 (IPv4,DNS)\n           reach : 0x00000002 (Reachable)\nNetwork interfaces: en0 utun7\n";
        let mut active = parse_macos_nwi_interfaces(nwi);
        let routes = "Routing tables\n\nInternet:\nDestination Gateway Flags Netif Expire\ndefault link#20 UCS utun7\n100.64/10 link#21 UCS utun8 !\nInternet6:\nDestination Gateway Flags Netif Expire\ndefault fe80::%utun0 UGcIg utun0\nfe80::%utun0/64 fe80::1 UcI utun0\nff00::/8 ::1 UmCI utun1\n";
        active.extend(parse_macos_routed_interfaces(routes));
        assert!(active.contains("en0"));
        assert!(active.contains("utun7"));
        assert!(active.contains("utun8"));
        assert!(!active.contains("utun0"));
        assert!(!active.contains("utun1"));
    }

    #[test]
    fn active_total_excludes_apple_peer_and_dormant_tunnels() {
        assert!(is_user_uplink("en0", "Wi-Fi", true, true));
        assert!(!is_user_uplink("awdl0", "Unknown", false, false));
        assert!(!is_user_uplink("utun7", "VPN/Tunnel", true, false));
        assert!(!is_user_uplink("lo0", "Loopback", false, false));
        assert!(!has_non_link_local_address(&["fe80::1".to_string()]));
        assert!(has_non_link_local_address(&["10.1.0.91".to_string()]));
        assert!(has_non_link_local_address(&[
            "fd48:7b1c:8406::1".to_string()
        ]));
    }

    #[test]
    fn hardware_mapped_default_thunderbolt_bridge_is_a_physical_uplink() {
        assert_eq!(classify_hardware_port("Thunderbolt Bridge"), "Ethernet");
        assert!(is_user_uplink("bridge0", "Ethernet", true, true));
        assert!(!is_user_uplink("bridge0", "Ethernet", false, true));
        assert!(!is_user_uplink("bridge0", "Ethernet", true, false));
    }

    #[test]
    fn windows_default_adapter_aliases_match_friendly_sysinfo_name() {
        let aliases = InterfaceAliases {
            values: vec![
                "{89ABCDEF-0123-4567-89AB-CDEF01234567}".to_string(),
                "Wi-Fi".to_string(),
                "Intel(R) Wi-Fi 7 BE200".to_string(),
            ],
        };
        assert!(aliases.matches("wi-fi"));
        assert!(aliases.matches(r"\DEVICE\TCPIP_{89ABCDEF-0123-4567-89AB-CDEF01234567}"));
        assert!(!aliases.matches("Ethernet"));

        let details = [
            InterfaceInfo {
                name: "Ethernet".to_string(),
                mac: "N/A".to_string(),
                ip_addresses: vec!["192.0.2.2".to_string()],
                is_up: true,
                interface_type: "Ethernet".to_string(),
                rx_bytes: 0,
                tx_bytes: 0,
            },
            InterfaceInfo {
                name: "Wi-Fi".to_string(),
                mac: "N/A".to_string(),
                ip_addresses: vec!["192.0.2.3".to_string()],
                is_up: true,
                interface_type: "Wi-Fi".to_string(),
                rx_bytes: 0,
                tx_bytes: 0,
            },
        ];
        let selected = details
            .iter()
            .find(|interface| interface.is_up && aliases.matches(&interface.name));
        assert_eq!(
            selected.map(|interface| interface.name.as_str()),
            Some("Wi-Fi")
        );
    }

    #[test]
    fn windows_virtual_and_vpn_adapters_are_not_physical_uplinks() {
        for name in [
            "vEthernet (Default Switch)",
            "vEthernet (WSL)",
            "Hyper-V Virtual Ethernet Adapter",
            "VMware Network Adapter VMnet8",
            "VirtualBox Host-Only Network",
            "DockerNAT",
        ] {
            assert_eq!(detect_interface_type(name), "Virtual", "{name}");
            assert!(!is_user_uplink(name, "Virtual", true, false), "{name}");
        }

        for name in [
            "Tailscale",
            "Wintun Userspace Tunnel",
            "WireGuard Tunnel",
            "NordLynx Tunnel",
            "TAP-Windows Adapter V9",
            "OpenVPN Data Channel Offload",
            "Cisco AnyConnect Secure Mobility Client Connection",
            "PANGP Virtual Ethernet Adapter #2",
            "GlobalProtect",
            "Fortinet SSL VPN Virtual Ethernet Adapter",
        ] {
            assert_eq!(detect_interface_type(name), "VPN/Tunnel", "{name}");
            assert!(!is_user_uplink(name, "VPN/Tunnel", true, false), "{name}");
        }

        for name in [
            "Ethernet",
            "Intel(R) Ethernet Connection",
            "Realtek PCIe GbE Family Controller",
        ] {
            assert_eq!(detect_interface_type(name), "Ethernet", "{name}");
            assert!(is_user_uplink(name, "Ethernet", false, false), "{name}");
        }
    }

    #[test]
    fn operational_flags_are_independent_of_traffic_and_addresses() {
        assert!(operational_from_flags(0x1, false));
        assert!(operational_from_flags(0x8863, false));
        assert!(!operational_from_flags(0, false));
        assert!(!operational_from_flags(0x1, true));
        assert!(operational_from_flags(0x1 | 0x40, true));
    }

    #[test]
    fn tunnel_summary_retains_underlying_uplink() {
        assert_eq!(describe_tunnel_over(&["Wi-Fi"]), "VPN/Tunnel over Wi-Fi");
        assert_eq!(describe_tunnel_over(&[]), "VPN/Tunnel");
        assert_eq!(
            describe_tunnel_over(&["Wi-Fi", "Ethernet"]),
            "VPN/Tunnel over ambiguous active uplinks (Ethernet, Wi-Fi)"
        );
    }
}
