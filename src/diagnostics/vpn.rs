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
        collect_macos_ifconfig(&mut vpns).await;
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
        collect_macos_ifconfig(&mut vpns).await;
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
    if let Ok(output) = tokio::process::Command::new("ipconfig")
        .args(["/all"])
        .output()
        .await
    {
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
async fn collect_macos_ifconfig(vpns: &mut Vec<VpnAdapter>) {
    if let Ok(output) = tokio::process::Command::new("ifconfig").output().await {
        let text = String::from_utf8_lossy(&output.stdout);
        let mut current_iface = String::new();
        let mut current_ip = None;

        for line in text.lines() {
            if !line.starts_with('\t') && !line.starts_with(' ') {
                if !current_iface.is_empty() && is_vpn_interface(&current_iface) {
                    let vendor = detect_vendor(&current_iface);
                    let is_enterprise = is_enterprise_vendor(&current_iface, vendor.as_deref());
                    vpns.push(VpnAdapter {
                        name: current_iface.clone(),
                        adapter_type: detect_vpn_type(&current_iface),
                        status: if current_ip.is_some() {
                            "Connected"
                        } else {
                            "Disconnected"
                        }
                        .to_string(),
                        ip_address: current_ip.take(),
                        vendor,
                        is_enterprise,
                        interface_name: Some(current_iface.clone()),
                    });
                }
                current_iface = line.split(':').next().unwrap_or("").to_string();
                current_ip = None;
            } else if line.contains("inet ") {
                current_ip = line.split_whitespace().nth(1).map(|s| s.to_string());
            }
        }

        if !current_iface.is_empty() && is_vpn_interface(&current_iface) {
            let vendor = detect_vendor(&current_iface);
            let is_enterprise = is_enterprise_vendor(&current_iface, vendor.as_deref());
            vpns.push(VpnAdapter {
                name: current_iface.clone(),
                adapter_type: detect_vpn_type(&current_iface),
                status: if current_ip.is_some() {
                    "Connected"
                } else {
                    "Disconnected"
                }
                .to_string(),
                ip_address: current_ip,
                vendor,
                is_enterprise,
                interface_name: Some(current_iface),
            });
        }
    }
}

#[cfg(target_os = "macos")]
async fn collect_macos_scutil(vpns: &mut Vec<VpnAdapter>) {
    if let Ok(output) = tokio::process::Command::new("scutil")
        .args(["--nc", "list"])
        .output()
        .await
    {
        let text = String::from_utf8_lossy(&output.stdout);
        for line in text.lines() {
            // Lines like: * (Connected)      "VPN Name" [L2TP]
            //   or:      - (Disconnected)   "VPN Name" [IPSec]
            let trimmed = line.trim();
            let status = if trimmed.contains("(Connected)") {
                "Connected"
            } else if trimmed.contains("(Disconnected)") {
                "Disconnected"
            } else {
                continue;
            };

            // Extract name between quotes
            if let (Some(start), Some(end)) = (trimmed.find('"'), trimmed.rfind('"')) {
                if start < end {
                    let name = &trimmed[start + 1..end];

                    // Skip if already found via ifconfig
                    if vpns.iter().any(|v| v.name == name) {
                        continue;
                    }

                    // Extract type in brackets
                    let vpn_type = if let Some(bracket_start) = trimmed.rfind('[') {
                        if let Some(bracket_end) = trimmed.rfind(']') {
                            trimmed[bracket_start + 1..bracket_end].to_string()
                        } else {
                            "VPN".to_string()
                        }
                    } else {
                        "VPN".to_string()
                    };

                    let vendor = detect_vendor(name);
                    let is_enterprise = is_enterprise_vendor(name, vendor.as_deref());

                    vpns.push(VpnAdapter {
                        name: name.to_string(),
                        adapter_type: vpn_type,
                        status: status.to_string(),
                        ip_address: None,
                        vendor,
                        is_enterprise,
                        interface_name: None,
                    });
                }
            }
        }
    }
}

// ── Linux ───────────────────────────────────────────────────────────────────

#[cfg(target_os = "linux")]
async fn collect_linux_ip_link(vpns: &mut Vec<VpnAdapter>) {
    if let Ok(output) = tokio::process::Command::new("ip")
        .args(["link", "show"])
        .output()
        .await
    {
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
    if let Ok(output) = tokio::process::Command::new("nmcli")
        .args([
            "-t",
            "-f",
            "TYPE,NAME,DEVICE",
            "connection",
            "show",
            "--active",
        ])
        .output()
        .await
    {
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
    if let Ok(output) = tokio::process::Command::new("wg")
        .args(["show", "interfaces"])
        .output()
        .await
    {
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

#[cfg(unix)]
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
