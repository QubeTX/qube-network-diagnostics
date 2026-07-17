use serde::Serialize;

use super::shared_cache::SharedCache;

#[derive(Debug, Clone, Serialize)]
pub struct VpnAdapter {
    pub name: String,
    pub adapter_type: String,
    pub status: String,
    pub ip_address: Option<String>,
    pub vendor: Option<String>,
    pub is_enterprise: bool,
    pub interface_name: Option<String>,
}

pub async fn collect_with_cache(cache: &SharedCache) -> Option<Vec<VpnAdapter>> {
    let mut vpns = Vec::new();

    #[cfg(windows)]
    {
        if let Some(ref ic) = cache.ipconfig {
            parse_vpn_from_ipconfig(&ic.raw, &mut vpns);
        } else {
            collect_windows_ipconfig(&mut vpns).await;
        }
        collect_windows_wmi(&mut vpns).await;
    }

    #[cfg(target_os = "macos")]
    {
        let _ = cache;
        let active_interfaces = collect_macos_active_interfaces().await;
        collect_macos_ifconfig(&mut vpns, &active_interfaces).await;
        collect_macos_scutil(&mut vpns).await;
    }

    #[cfg(target_os = "linux")]
    {
        let _ = cache;
        collect_linux_ip_link(&mut vpns).await;
        collect_linux_nmcli(&mut vpns).await;
        collect_linux_wireguard(&mut vpns).await;
    }

    vpns.dedup_by(|a, b| {
        if let (Some(ref ai), Some(ref bi)) = (&a.interface_name, &b.interface_name) {
            ai == bi
        } else {
            a.name == b.name
        }
    });

    if vpns.is_empty() {
        None
    } else {
        Some(vpns)
    }
}

pub async fn collect() -> Option<Vec<VpnAdapter>> {
    let mut vpns = Vec::new();

    #[cfg(windows)]
    {
        collect_windows_ipconfig(&mut vpns).await;
        collect_windows_wmi(&mut vpns).await;
    }

    #[cfg(target_os = "macos")]
    {
        let active_interfaces = collect_macos_active_interfaces().await;
        collect_macos_ifconfig(&mut vpns, &active_interfaces).await;
        collect_macos_scutil(&mut vpns).await;
    }

    #[cfg(target_os = "linux")]
    {
        collect_linux_ip_link(&mut vpns).await;
        collect_linux_nmcli(&mut vpns).await;
        collect_linux_wireguard(&mut vpns).await;
    }

    // Deduplicate by interface name
    vpns.dedup_by(|a, b| {
        if let (Some(ref ai), Some(ref bi)) = (&a.interface_name, &b.interface_name) {
            ai == bi
        } else {
            a.name == b.name
        }
    });

    if vpns.is_empty() {
        None
    } else {
        Some(vpns)
    }
}

// ── Windows ─────────────────────────────────────────────────────────────────

#[cfg(windows)]
fn parse_vpn_from_ipconfig(text: &str, vpns: &mut Vec<VpnAdapter>) {
    let mut current_name = String::new();
    let mut current_ip = None;

    for line in text.lines() {
        if !line.starts_with(' ') && !line.starts_with('\t') && line.contains("adapter") {
            let name = line.trim().trim_end_matches(':');
            let lower = name.to_lowercase();
            if lower.contains("vpn")
                || lower.contains("tap")
                || lower.contains("tun")
                || lower.contains("wireguard")
                || lower.contains("wintun")
                || lower.contains("fortinet")
                || lower.contains("cisco")
                || lower.contains("palo alto")
                || lower.contains("global protect")
                || lower.contains("nordlynx")
                || lower.contains("expressvpn")
                || lower.contains("mullvad")
                || lower.contains("tailscale")
                || lower.contains("zscaler")
                || lower.contains("pulse")
            {
                if !current_name.is_empty() {
                    let vendor = detect_vendor(&current_name);
                    let is_enterprise = is_enterprise_vendor(&current_name, vendor.as_deref());
                    vpns.push(VpnAdapter {
                        name: current_name.clone(),
                        adapter_type: detect_vpn_type(&current_name),
                        status: if current_ip.is_some() {
                            "Connected"
                        } else {
                            "Disconnected"
                        }
                        .to_string(),
                        ip_address: current_ip.take(),
                        vendor,
                        is_enterprise,
                        interface_name: None,
                    });
                }
                current_name = name.to_string();
                current_ip = None;
            } else {
                current_name.clear();
            }
        } else if !current_name.is_empty() {
            let trimmed = line.trim();
            if trimmed.contains("IPv4 Address")
                || (trimmed.contains("IP Address") && !trimmed.contains("Autoconfiguration"))
            {
                current_ip = trimmed
                    .split(':')
                    .nth(1)
                    .map(|s| s.trim().trim_end_matches("(Preferred)").trim().to_string());
            }
        }
    }

    if !current_name.is_empty() {
        let vendor = detect_vendor(&current_name);
        let is_enterprise = is_enterprise_vendor(&current_name, vendor.as_deref());
        vpns.push(VpnAdapter {
            name: current_name.clone(),
            adapter_type: detect_vpn_type(&current_name),
            status: if current_ip.is_some() {
                "Connected"
            } else {
                "Disconnected"
            }
            .to_string(),
            ip_address: current_ip,
            vendor,
            is_enterprise,
            interface_name: None,
        });
    }
}

#[cfg(windows)]
async fn collect_windows_ipconfig(vpns: &mut Vec<VpnAdapter>) {
    let mut cmd = tokio::process::Command::new("ipconfig");
    cmd.args(["/all"]);
    if let Some(output) = super::util::run_with_timeout(cmd, super::util::QUICK).await {
        let text = String::from_utf8_lossy(&output.stdout);
        parse_vpn_from_ipconfig(&text, vpns);
    }
}

#[cfg(windows)]
async fn collect_windows_wmi(vpns: &mut Vec<VpnAdapter>) {
    use std::collections::HashMap;
    use wmi::{COMLibrary, WMIConnection};

    // Extract into Send-safe tuple inside spawn_blocking
    let wmi_rows: Vec<(String, Option<String>, u16)> = tokio::task::spawn_blocking(|| {
        let com = match COMLibrary::new() {
            Ok(c) => c,
            Err(_) => return Vec::new(),
        };
        let wmi = match WMIConnection::new(com) {
            Ok(w) => w,
            Err(_) => return Vec::new(),
        };

        let query = r#"SELECT Name, NetConnectionID, Description, NetConnectionStatus FROM Win32_NetworkAdapter WHERE Description LIKE '%TAP%' OR Description LIKE '%TUN%' OR Description LIKE '%Wintun%' OR Description LIKE '%WireGuard%' OR Description LIKE '%VPN%' OR Description LIKE '%NordLynx%' OR Description LIKE '%ExpressVPN%' OR Description LIKE '%Tailscale%'"#;
        let results: Vec<HashMap<String, wmi::Variant>> = wmi.raw_query(query).unwrap_or_default();

        results.into_iter().filter_map(|row| {
            let description = match row.get("Description") {
                Some(wmi::Variant::String(s)) => s.clone(),
                _ => return None,
            };
            let net_id = match row.get("NetConnectionID") {
                Some(wmi::Variant::String(s)) => Some(s.clone()),
                _ => None,
            };
            let status_val = match row.get("NetConnectionStatus") {
                Some(wmi::Variant::UI2(n)) => *n,
                Some(wmi::Variant::I4(n)) => *n as u16,
                _ => 0,
            };
            Some((description, net_id, status_val))
        }).collect()
    })
    .await
    .unwrap_or_default();

    for (description, net_id, status_val) in wmi_rows {
        // Skip if we already have this from ipconfig
        let name_for_check = net_id.clone().unwrap_or_else(|| description.clone());
        if vpns
            .iter()
            .any(|v| v.name == name_for_check || v.name == description)
        {
            continue;
        }

        let vendor = detect_vendor(&description);
        let is_enterprise = is_enterprise_vendor(&description, vendor.as_deref());

        vpns.push(VpnAdapter {
            name: name_for_check,
            adapter_type: detect_vpn_type(&description),
            status: if status_val == 2 {
                "Connected"
            } else {
                "Disconnected"
            }
            .to_string(),
            ip_address: None,
            vendor,
            is_enterprise,
            interface_name: net_id,
        });
    }
}

// ── macOS ───────────────────────────────────────────────────────────────────

#[cfg(target_os = "macos")]
async fn collect_macos_active_interfaces() -> std::collections::HashSet<String> {
    let mut cmd = tokio::process::Command::new("/usr/sbin/scutil");
    cmd.arg("--nwi");
    let mut routes4 = tokio::process::Command::new("netstat");
    routes4.args(["-rn", "-f", "inet"]);
    let mut routes6 = tokio::process::Command::new("netstat");
    routes6.args(["-rn", "-f", "inet6"]);
    let (nwi, routes4, routes6) = tokio::join!(
        super::util::run_with_timeout(cmd, super::util::QUICK),
        super::util::run_with_timeout(routes4, super::util::QUICK),
        super::util::run_with_timeout(routes6, super::util::QUICK),
    );
    let mut active = nwi
        .map(|output| parse_macos_nwi(&String::from_utf8_lossy(&output.stdout)))
        .unwrap_or_default();
    for output in [routes4, routes6].into_iter().flatten() {
        active.extend(parse_macos_route_interfaces(&String::from_utf8_lossy(
            &output.stdout,
        )));
    }
    active
}

#[cfg(any(target_os = "macos", test))]
fn parse_macos_nwi(text: &str) -> std::collections::HashSet<String> {
    let mut active = std::collections::HashSet::new();
    for line in text.lines().map(str::trim) {
        if let Some(interfaces) = line.strip_prefix("Network interfaces:") {
            active.extend(interfaces.split_whitespace().map(str::to_string));
        } else if line.contains(": flags") && line.contains("Reachable") {
            if let Some((name, _)) = line.split_once(':') {
                active.insert(name.trim().to_string());
            }
        }
    }
    active
}

#[cfg(any(target_os = "macos", test))]
fn parse_macos_route_interfaces(text: &str) -> std::collections::HashSet<String> {
    text.lines()
        .filter_map(|line| {
            let fields: Vec<&str> = line.split_whitespace().collect();
            (fields.len() >= 4
                && !matches!(
                    fields[0],
                    "Destination" | "Routing" | "Internet:" | "Internet6:"
                )
                && meaningful_macos_route(fields[0], fields[1], fields[2]))
            .then(|| fields[3].to_string())
        })
        .collect()
}

#[cfg(any(target_os = "macos", test))]
fn meaningful_macos_route(destination: &str, gateway: &str, _flags: &str) -> bool {
    let destination = destination.to_ascii_lowercase();
    let gateway = gateway.to_ascii_lowercase();

    if destination == "default" {
        // Modern macOS installs interface-scoped IPv6 defaults for every utun,
        // including dormant system tunnels. A reachable VPN is also present in
        // `scutil --nwi`, so a link-local gateway is not sufficient route
        // evidence on its own.
        return !gateway.starts_with("fe80::");
    }

    !(destination.starts_with("fe80")
        || destination.starts_with("ff")
        || destination.starts_with("169.254")
        || destination.starts_with("::1")
        || destination.starts_with("link#"))
}

#[cfg(target_os = "macos")]
async fn collect_macos_ifconfig(
    vpns: &mut Vec<VpnAdapter>,
    active_interfaces: &std::collections::HashSet<String>,
) {
    let cmd = tokio::process::Command::new("ifconfig");
    if let Some(output) = super::util::run_with_timeout(cmd, super::util::QUICK).await {
        let text = String::from_utf8_lossy(&output.stdout);
        vpns.extend(parse_macos_ifconfig(&text, active_interfaces));
    }
}

#[cfg(any(target_os = "macos", test))]
fn parse_macos_ifconfig(
    text: &str,
    active_interfaces: &std::collections::HashSet<String>,
) -> Vec<VpnAdapter> {
    let mut rows = Vec::new();
    let mut current_iface = String::new();
    let mut current_ip: Option<String> = None;

    let flush = |rows: &mut Vec<VpnAdapter>, iface: &str, ip: Option<String>| {
        if !is_vpn_interface(iface) {
            return;
        }
        // Darwin leaves many utun devices UP for system services, sometimes
        // even with non-link-local addresses. Require reachability from
        // `scutil --nwi` or ownership of an IPv4/IPv6 route; an address alone
        // is not evidence of a currently active VPN.
        if !active_interfaces.contains(iface) {
            return;
        }
        let vendor = detect_vendor(iface);
        let is_enterprise = is_enterprise_vendor(iface, vendor.as_deref());
        rows.push(VpnAdapter {
            name: iface.to_string(),
            adapter_type: detect_vpn_type(iface),
            status: "Connected".to_string(),
            ip_address: ip,
            vendor,
            is_enterprise,
            interface_name: Some(iface.to_string()),
        });
    };

    for line in text.lines() {
        if !line.starts_with(char::is_whitespace) {
            flush(&mut rows, &current_iface, current_ip.take());
            current_iface = line
                .split_once(':')
                .map(|(name, _)| name)
                .unwrap_or("")
                .to_string();
        } else {
            let trimmed = line.trim();
            if let Some(value) = trimmed.strip_prefix("inet ") {
                let address = value.split_whitespace().next().unwrap_or("");
                if !address.starts_with("127.") && !address.is_empty() {
                    current_ip = Some(address.to_string());
                }
            } else if let Some(value) = trimmed.strip_prefix("inet6 ") {
                let address = value.split_whitespace().next().unwrap_or("");
                if !address.starts_with("fe80:")
                    && address != "::1"
                    && !address.is_empty()
                    && current_ip.is_none()
                {
                    current_ip = Some(address.to_string());
                }
            }
        }
    }
    flush(&mut rows, &current_iface, current_ip);
    rows
}

#[cfg(target_os = "macos")]
async fn collect_macos_scutil(vpns: &mut Vec<VpnAdapter>) {
    let mut cmd = tokio::process::Command::new("scutil");
    cmd.args(["--nc", "list"]);
    if let Some(output) = super::util::run_with_timeout(cmd, super::util::QUICK).await {
        let text = String::from_utf8_lossy(&output.stdout);
        for row in parse_macos_scutil(&text) {
            // Skip if already found via ifconfig.
            if !vpns.iter().any(|vpn| vpn.name == row.name) {
                vpns.push(row);
            }
        }
    }
}

#[cfg(any(target_os = "macos", test))]
fn parse_macos_scutil(text: &str) -> Vec<VpnAdapter> {
    text.lines()
        .filter_map(|line| {
            // Current NetworkExtension services include an application ID
            // before the quoted display name; older L2TP/IPSec rows do not.
            let trimmed = line.trim();
            let status = if trimmed.contains("(Connected)") {
                "Connected"
            } else if trimmed.contains("(Disconnected)") {
                "Disconnected"
            } else {
                return None;
            };

            let (start, end) = (trimmed.find('"')?, trimmed.rfind('"')?);
            (start < end).then(|| {
                let name = &trimmed[start + 1..end];
                let adapter_type = trimmed
                    .rfind('[')
                    .zip(trimmed.rfind(']'))
                    .filter(|(open, close)| open < close)
                    .map(|(open, close)| trimmed[open + 1..close].to_string())
                    .unwrap_or_else(|| "VPN".to_string());
                let vendor = detect_vendor(name);
                let is_enterprise = is_enterprise_vendor(name, vendor.as_deref());
                VpnAdapter {
                    name: name.to_string(),
                    adapter_type,
                    status: status.to_string(),
                    ip_address: None,
                    vendor,
                    is_enterprise,
                    interface_name: None,
                }
            })
        })
        .collect()
}

// ── Linux ───────────────────────────────────────────────────────────────────

#[cfg(target_os = "linux")]
async fn collect_linux_ip_link(vpns: &mut Vec<VpnAdapter>) {
    let mut cmd = tokio::process::Command::new("ip");
    cmd.args(["link", "show"]);
    if let Some(output) = super::util::run_with_timeout(cmd, super::util::QUICK).await {
        let text = String::from_utf8_lossy(&output.stdout);
        for line in text.lines() {
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() >= 2 {
                let name = parts[1].trim_end_matches(':');
                if is_vpn_interface(name) {
                    let is_up = line.contains("state UP");
                    let vendor = detect_vendor(name);
                    let is_enterprise = is_enterprise_vendor(name, vendor.as_deref());
                    vpns.push(VpnAdapter {
                        name: name.to_string(),
                        adapter_type: detect_vpn_type(name),
                        status: if is_up { "Connected" } else { "Disconnected" }.to_string(),
                        ip_address: None,
                        vendor,
                        is_enterprise,
                        interface_name: Some(name.to_string()),
                    });
                }
            }
        }
    }
}

#[cfg(target_os = "linux")]
async fn collect_linux_nmcli(vpns: &mut Vec<VpnAdapter>) {
    let mut cmd = tokio::process::Command::new("nmcli");
    cmd.args([
        "-t",
        "-f",
        "TYPE,NAME,DEVICE",
        "connection",
        "show",
        "--active",
    ]);
    if let Some(output) = super::util::run_with_timeout(cmd, super::util::QUICK).await {
        let text = String::from_utf8_lossy(&output.stdout);
        for line in text.lines() {
            let parts: Vec<&str> = line.splitn(3, ':').collect();
            if parts.len() >= 2 {
                let conn_type = parts[0];
                let conn_name = parts[1];
                let device = if parts.len() >= 3 {
                    Some(parts[2])
                } else {
                    None
                };

                // NM VPN connection types
                if conn_type.contains("vpn") || conn_type.contains("wireguard") {
                    if vpns.iter().any(|v| v.name == conn_name) {
                        continue;
                    }
                    let vendor = detect_vendor(conn_name);
                    let is_enterprise = is_enterprise_vendor(conn_name, vendor.as_deref());
                    vpns.push(VpnAdapter {
                        name: conn_name.to_string(),
                        adapter_type: conn_type.to_string(),
                        status: "Connected".to_string(),
                        ip_address: None,
                        vendor,
                        is_enterprise,
                        interface_name: device.map(|d| d.to_string()),
                    });
                }
            }
        }
    }
}

#[cfg(target_os = "linux")]
async fn collect_linux_wireguard(vpns: &mut Vec<VpnAdapter>) {
    let mut cmd = tokio::process::Command::new("wg");
    cmd.args(["show", "interfaces"]);
    if let Some(output) = super::util::run_with_timeout(cmd, super::util::QUICK).await {
        if output.status.success() {
            let text = String::from_utf8_lossy(&output.stdout);
            for iface in text.split_whitespace() {
                if vpns
                    .iter()
                    .any(|v| v.interface_name.as_deref() == Some(iface))
                {
                    continue;
                }
                vpns.push(VpnAdapter {
                    name: iface.to_string(),
                    adapter_type: "WireGuard".to_string(),
                    status: "Connected".to_string(),
                    ip_address: None,
                    vendor: Some("WireGuard".to_string()),
                    is_enterprise: false,
                    interface_name: Some(iface.to_string()),
                });
            }
        }
    }
}

// ── Shared helpers ──────────────────────────────────────────────────────────

#[cfg(any(unix, test))]
fn is_vpn_interface(name: &str) -> bool {
    let lower = name.to_lowercase();
    lower.starts_with("tun")
        || lower.starts_with("tap")
        || lower.starts_with("utun")
        || lower.starts_with("wg")
        || lower.starts_with("ppp")
        || lower.contains("vpn")
        || lower.contains("wireguard")
        || lower.contains("wintun")
}

fn detect_vpn_type(name: &str) -> String {
    let lower = name.to_lowercase();
    if lower.contains("wireguard") || lower.starts_with("wg") || lower.contains("wintun") {
        "WireGuard".to_string()
    } else if lower.starts_with("tun") || lower.starts_with("utun") {
        "TUN Tunnel".to_string()
    } else if lower.starts_with("tap") {
        "TAP Tunnel".to_string()
    } else if lower.starts_with("ppp") {
        "PPP".to_string()
    } else if lower.contains("cisco") || lower.contains("anyconnect") {
        "Cisco AnyConnect".to_string()
    } else if lower.contains("fortinet") || lower.contains("forticlient") {
        "FortiClient".to_string()
    } else if lower.contains("global protect")
        || lower.contains("globalprotect")
        || lower.contains("palo alto")
    {
        "GlobalProtect".to_string()
    } else if lower.contains("zscaler") {
        "Zscaler".to_string()
    } else if lower.contains("pulse") {
        "Pulse Secure".to_string()
    } else {
        "VPN".to_string()
    }
}

fn detect_vendor(name: &str) -> Option<String> {
    let lower = name.to_lowercase();
    if lower.contains("nord") || lower.contains("nordlynx") {
        Some("NordVPN".to_string())
    } else if lower.contains("expressvpn") {
        Some("ExpressVPN".to_string())
    } else if lower.contains("mullvad") {
        Some("Mullvad".to_string())
    } else if lower.contains("tailscale") {
        Some("Tailscale".to_string())
    } else if lower.contains("wireguard") || lower.starts_with("wg") {
        Some("WireGuard".to_string())
    } else if lower.contains("cisco") || lower.contains("anyconnect") {
        Some("Cisco".to_string())
    } else if lower.contains("globalprotect")
        || lower.contains("global protect")
        || lower.contains("palo alto")
    {
        Some("Palo Alto".to_string())
    } else if lower.contains("fortinet") || lower.contains("forticlient") {
        Some("Fortinet".to_string())
    } else if lower.contains("zscaler") {
        Some("Zscaler".to_string())
    } else if lower.contains("pulse") {
        Some("Pulse Secure".to_string())
    } else {
        None
    }
}

fn is_enterprise_vendor(name: &str, vendor: Option<&str>) -> bool {
    let lower = name.to_lowercase();
    let vendor_lower = vendor.unwrap_or("").to_lowercase();

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
        .any(|p| lower.contains(p) || vendor_lower.contains(p))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn macos_nwi_identifies_only_reachable_interfaces() {
        let text = "IPv4 network interface information\n     en0 : flags : 0x5 (IPv4,DNS)\n           reach : 0x00000002 (Reachable)\nNetwork interfaces: en0 utun7\n";
        let active = parse_macos_nwi(text);
        assert!(active.contains("en0"));
        assert!(active.contains("utun7"));
        assert!(!active.contains("utun0"));
    }

    #[test]
    fn macos_routes_identify_split_and_default_tunnels() {
        let text = "Routing tables\n\nInternet:\nDestination Gateway Flags Netif Expire\ndefault link#20 UCS utun7\n100.64/10 link#21 UCS utun8 !\nInternet6:\nDestination Gateway Flags Netif Expire\ndefault fe80::%utun0 UGcIg utun0\nfe80::%utun0/64 fe80::1 UcI utun0\nff00::/8 ::1 UmCI utun1\n";
        let routed = parse_macos_route_interfaces(text);
        assert!(routed.contains("utun7"));
        assert!(routed.contains("utun8"));
        assert!(!routed.contains("!"));
        assert!(!routed.contains("utun0"));
        assert!(!routed.contains("utun1"));
    }

    #[test]
    fn dormant_link_local_utuns_are_not_reported_as_vpns() {
        let text = "utun0: flags=8051<UP,POINTOPOINT,RUNNING> mtu 1380\n\tinet6 fe80::1%utun0 prefixlen 64\nutun7: flags=8051<UP,POINTOPOINT,RUNNING> mtu 1380\n\tinet6 fe80::2%utun7 prefixlen 64\nutun8: flags=8051<UP,POINTOPOINT,RUNNING> mtu 1380\n\tinet 100.64.0.2 --> 100.64.0.1 netmask 0xffffffff\n";
        let active = ["utun7".to_string()].into_iter().collect();
        let rows = parse_macos_ifconfig(text, &active);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].interface_name.as_deref(), Some("utun7"));
    }

    #[test]
    fn macos_scutil_parses_modern_network_extension_service() {
        let text = "Available network connection services in the current set (*=enabled):\n* (Disconnected) 38349A36 VPN (io.tailscale.ipn.macos) \"Tailscale\" [VPN:io.tailscale.ipn.macos]\n";
        let rows = parse_macos_scutil(text);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].name, "Tailscale");
        assert_eq!(rows[0].status, "Disconnected");
        assert_eq!(rows[0].vendor.as_deref(), Some("Tailscale"));
    }
}
