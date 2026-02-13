use std::collections::BTreeSet;

use serde::Serialize;

use super::DiagnosticResult;

#[derive(Debug, Clone, Serialize)]
pub struct AdapterInfo {
    pub name: String,
    pub adapter_type: String,
    pub status: String,
    pub has_ip: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mac_address: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub link_speed_mbps: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rx_link_speed_mbps: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dns_servers: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gateways: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub media_connect_state: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub physical_medium: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mtu: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ipv4_metric: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub driver_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub driver_version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub driver_date: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub problem_code: Option<u32>,
}

fn display_type(adapter_type: &str, name: &str, physical_medium: Option<&str>) -> &'static str {
    // Priority 1: PhysicalMediumType from GetIfEntry2 (most accurate)
    if let Some(pm) = physical_medium {
        match pm {
            "Native802_11" | "WirelessLan" => return "Wi-Fi",
            "Bluetooth" => return "Bluetooth",
            "BluetoothPAN" => return "BT PAN",
            "Ethernet802_3" => return "Ethernet",
            "WirelessWan" => return "WWAN",
            _ => {} // fall through
        }
    }

    // Priority 2: Name-based heuristics
    let lower_name = name.to_lowercase();

    if lower_name.contains("virtualbox")
        || lower_name.contains("vmware")
        || lower_name.contains("hyper-v")
        || lower_name.contains("vethernet")
        || lower_name.contains("docker")
        || lower_name.contains("virtual")
        || lower_name.contains("host-only")
        || lower_name.contains("vm network")
    {
        return "Virtual";
    }
    if lower_name.contains("wi-fi")
        || lower_name.contains("wireless")
        || lower_name.contains("wlan")
    {
        return "Wi-Fi";
    }
    if lower_name.contains("bluetooth") {
        if lower_name.contains("personal area network")
            || lower_name.contains("pan")
            || lower_name.contains("bnep")
        {
            return "BT PAN";
        }
        return "Bluetooth";
    }

    // Priority 3: adapter_type field (IfType-derived)
    match adapter_type {
        "Ieee80211" | "Wi-Fi" => "Wi-Fi",
        t if t.contains("802.11") || t.to_lowercase().contains("wireless") => "Wi-Fi",
        "Bluetooth" => "Bluetooth",
        "EthernetCsmacd" | "Ethernet" => "Ethernet",
        t if t.contains("802.3") || t.eq_ignore_ascii_case("ethernet") => "Ethernet",
        "Tunnel" | "VPN/Tunnel" => "VPN",
        "Virtual" => "Virtual",
        "Ppp" => "PPP",
        "Other" | "Unknown" => "Other",
        _ => "Other",
    }
}

/// Build a set of display types for adapters matching a given status,
/// optionally appending link speed for active Wi-Fi adapters.
fn types_by_status(adapters: &[AdapterInfo], status: &str) -> BTreeSet<String> {
    adapters
        .iter()
        .filter(|a| a.status == status)
        .map(|a| {
            let dtype = display_type(
                &a.adapter_type,
                &a.name,
                a.physical_medium.as_deref(),
            );
            // Append link speed for active Wi-Fi adapters
            if status == "Active" && dtype == "Wi-Fi" {
                if let Some(speed) = a.link_speed_mbps {
                    if speed >= 1000 {
                        return format!("Wi-Fi {:.1} Gbps", speed as f64 / 1000.0);
                    } else if speed > 0 {
                        return format!("Wi-Fi {} Mbps", speed);
                    }
                }
            }
            dtype.to_string()
        })
        .collect()
}

fn format_types(types: &BTreeSet<String>) -> String {
    let v: Vec<&str> = types.iter().map(|s| s.as_str()).collect();
    v.join(", ")
}

pub async fn check() -> (DiagnosticResult, Vec<AdapterInfo>) {
    let adapters = collect_adapters().await;

    let error_count = adapters.iter().filter(|a| a.status == "Error").count();
    let disabled_count = adapters.iter().filter(|a| a.status == "Disabled").count();
    let active_count = adapters.iter().filter(|a| a.status == "Active").count();

    let result = if error_count > 0 {
        let names: Vec<&str> = adapters
            .iter()
            .filter(|a| a.status == "Error")
            .map(|a| a.name.as_str())
            .collect();
        DiagnosticResult::fail(
            "Adapters",
            format!(
                "{} adapter{} with errors: {}",
                error_count,
                if error_count > 1 { "s" } else { "" },
                names.join(", ")
            ),
        )
    } else if disabled_count > 0 && active_count > 0 {
        let active_types = types_by_status(&adapters, "Active");
        let disabled_types = types_by_status(&adapters, "Disabled");
        DiagnosticResult::warn(
            "Adapters",
            format!(
                "{} active ({})\n{} disabled ({})",
                active_count,
                format_types(&active_types),
                disabled_count,
                format_types(&disabled_types),
            ),
        )
    } else if active_count == 0 {
        DiagnosticResult::fail("Adapters", "No active network adapters found")
    } else {
        let active_types = types_by_status(&adapters, "Active");
        DiagnosticResult::ok(
            "Adapters",
            format!("{} active ({})", active_count, format_types(&active_types)),
        )
    };

    (result, adapters)
}

async fn collect_adapters() -> Vec<AdapterInfo> {
    #[cfg(windows)]
    {
        collect_adapters_windows().await
    }

    #[cfg(target_os = "macos")]
    {
        collect_adapters_macos().await
    }

    #[cfg(target_os = "linux")]
    {
        collect_adapters_linux().await
    }
}

// ── GetIfEntry2 helper (Windows only) ────────────────────────────────────

#[cfg(windows)]
struct IfEntry2Data {
    media_connect_state: u32, // 0=Unknown, 1=Connected, 2=Disconnected
    physical_medium_type: u32,
    admin_status: u32, // 1=Up, 2=Down, 3=Testing
    transmit_link_speed: u64,
    receive_link_speed: u64,
    mtu: u32,
}

#[cfg(windows)]
fn get_if_entry2(if_index: u32) -> Option<IfEntry2Data> {
    use std::mem::zeroed;
    use winapi::shared::netioapi::{GetIfEntry2, MIB_IF_ROW2};

    if if_index == 0 {
        return None;
    }

    unsafe {
        let mut row: MIB_IF_ROW2 = zeroed();
        row.InterfaceIndex = if_index;
        let ret = GetIfEntry2(&mut row);
        if ret != 0 {
            return None;
        }
        Some(IfEntry2Data {
            media_connect_state: row.MediaConnectState as u32,
            physical_medium_type: row.PhysicalMediumType as u32,
            admin_status: row.AdminStatus as u32,
            transmit_link_speed: row.TransmitLinkSpeed,
            receive_link_speed: row.ReceiveLinkSpeed,
            mtu: row.Mtu,
        })
    }
}

/// Map PhysicalMediumType integer to a string we use for classification.
#[cfg(windows)]
fn physical_medium_name(pm: u32) -> Option<&'static str> {
    // Values from NDIS_PHYSICAL_MEDIUM enum
    match pm {
        0 => None,           // Unspecified
        1 => Some("WirelessLan"),       // NdisPhysicalMediumWirelessLan
        2 => Some("CableModem"),
        3 => Some("PhoneLine"),
        4 => Some("PowerLine"),
        5 => Some("DSL"),
        6 => Some("FibreChannel"),
        7 => Some("1394"),              // IEEE 1394 / FireWire
        8 => Some("WirelessWan"),
        9 => Some("Native802_11"),      // NdisPhysicalMediumNative802_11
        10 => Some("Bluetooth"),
        11 => Some("Infiniband"),
        12 => Some("WiMax"),
        13 => Some("UWB"),
        14 => Some("Ethernet802_3"),    // NdisPhysicalMedium802_3
        _ => None,
    }
}

/// Derive adapter status from GetIfEntry2 + OperStatus.
#[cfg(windows)]
fn derive_status(
    oper_status: ipconfig::OperStatus,
    if2: Option<&IfEntry2Data>,
) -> String {
    if let Some(data) = if2 {
        // AdminStatus=2 means admin-disabled
        if data.admin_status == 2 {
            return "Disabled".to_string();
        }
        // Admin is up but media disconnected = no cable/signal
        if data.admin_status == 1 && data.media_connect_state == 2 {
            return "No Cable".to_string();
        }
    }

    match oper_status {
        ipconfig::OperStatus::IfOperStatusUp => "Active".to_string(),
        ipconfig::OperStatus::IfOperStatusDown => "Down".to_string(),
        ipconfig::OperStatus::IfOperStatusDormant => "Standby".to_string(),
        ipconfig::OperStatus::IfOperStatusNotPresent => "Not Present".to_string(),
        ipconfig::OperStatus::IfOperStatusLowerLayerDown => "Down".to_string(),
        _ => "Unknown".to_string(),
    }
}

#[cfg(windows)]
fn format_mac(bytes: &[u8]) -> String {
    bytes
        .iter()
        .map(|b| format!("{:02X}", b))
        .collect::<Vec<_>>()
        .join(":")
}

#[cfg(windows)]
async fn collect_adapters_windows() -> Vec<AdapterInfo> {
    tokio::task::spawn_blocking(|| {
        let raw_adapters = match ipconfig::get_adapters() {
            Ok(a) => a,
            Err(_) => return Vec::new(),
        };

        let mut adapters = Vec::new();

        for adapter in raw_adapters {
            // Skip loopback
            if adapter.if_type() == ipconfig::IfType::SoftwareLoopback {
                continue;
            }

            // Skip adapters with no MAC (virtual/software adapters)
            let mac = adapter.physical_address();
            let is_zero_mac = mac.map_or(true, |m| m.iter().all(|b| *b == 0));
            if is_zero_mac {
                continue;
            }

            // Get extended info from GetIfEntry2
            let if2 = get_if_entry2(adapter.ipv6_if_index());

            let if_type_str = match adapter.if_type() {
                ipconfig::IfType::EthernetCsmacd => "EthernetCsmacd",
                ipconfig::IfType::Ieee80211 => "Ieee80211",
                ipconfig::IfType::Tunnel => "Tunnel",
                ipconfig::IfType::Ppp => "Ppp",
                _ => "Other",
            };

            let oper_status = adapter.oper_status();
            let status = derive_status(oper_status, if2.as_ref());
            let has_ip = oper_status == ipconfig::OperStatus::IfOperStatusUp
                && !adapter.ip_addresses().is_empty();

            let physical_medium = if2
                .as_ref()
                .and_then(|d| physical_medium_name(d.physical_medium_type))
                .map(|s| s.to_string());

            let tx_speed_bps = if2.as_ref().map(|d| d.transmit_link_speed)
                .unwrap_or(adapter.transmit_link_speed());
            let rx_speed_bps = if2.as_ref().map(|d| d.receive_link_speed)
                .unwrap_or(adapter.receive_link_speed());

            let tx_mbps = tx_speed_bps / 1_000_000;
            let rx_mbps = rx_speed_bps / 1_000_000;

            let dns: Vec<String> = adapter.dns_servers().iter().map(|ip| ip.to_string()).collect();
            let gws: Vec<String> = adapter.gateways().iter().map(|ip| ip.to_string()).collect();

            let media_connect = if2.as_ref().map(|d| match d.media_connect_state {
                1 => "Connected".to_string(),
                2 => "Disconnected".to_string(),
                _ => "Unknown".to_string(),
            });

            adapters.push(AdapterInfo {
                name: adapter.friendly_name().to_string(),
                adapter_type: if_type_str.to_string(),
                status,
                has_ip,
                description: Some(adapter.description().to_string()),
                mac_address: mac.map(format_mac),
                link_speed_mbps: if tx_mbps > 0 { Some(tx_mbps) } else { None },
                rx_link_speed_mbps: if rx_mbps > 0 { Some(rx_mbps) } else { None },
                dns_servers: if dns.is_empty() { None } else { Some(dns) },
                gateways: if gws.is_empty() { None } else { Some(gws) },
                media_connect_state: media_connect,
                physical_medium,
                mtu: if2.as_ref().map(|d| d.mtu),
                ipv4_metric: Some(adapter.ipv4_metric()),
                driver_name: None,
                driver_version: None,
                driver_date: None,
                problem_code: None,
            });
        }

        adapters
    })
    .await
    .unwrap_or_default()
}

/// Enrich adapter list with driver info from WMI (tech mode only).
/// Matches by description (hardware chip name) which is more reliable
/// than the old name-based substring match.
#[cfg(windows)]
pub async fn enrich_driver_info(adapters: &mut Vec<AdapterInfo>) {
    use std::collections::HashMap;
    use wmi::{COMLibrary, WMIConnection};

    // Extract driver data inside the blocking closure into Send-safe types
    let driver_data: Vec<(String, Option<String>, Option<String>)> =
        tokio::task::spawn_blocking(|| {
            let com = match COMLibrary::new() {
                Ok(c) => c,
                Err(_) => return Vec::new(),
            };
            let wmi = match WMIConnection::new(com) {
                Ok(w) => w,
                Err(_) => return Vec::new(),
            };

            let query = "SELECT DeviceName, DriverVersion, DriverDate FROM Win32_PnPSignedDriver WHERE DeviceClass = 'NET'";
            let results: Vec<HashMap<String, wmi::Variant>> = match wmi.raw_query(query) {
                Ok(r) => r,
                Err(_) => return Vec::new(),
            };

            results
                .into_iter()
                .filter_map(|row| {
                    let name = match row.get("DeviceName") {
                        Some(wmi::Variant::String(s)) => s.clone(),
                        _ => return None,
                    };
                    let version = match row.get("DriverVersion") {
                        Some(wmi::Variant::String(s)) => Some(s.clone()),
                        _ => None,
                    };
                    let date = match row.get("DriverDate") {
                        Some(wmi::Variant::String(s)) => Some(s.chars().take(10).collect()),
                        _ => None,
                    };
                    Some((name, version, date))
                })
                .collect()
        })
        .await
        .unwrap_or_default();

    for (drv_name, version, date) in &driver_data {
        for adapter in adapters.iter_mut() {
            let matches = adapter
                .description
                .as_ref()
                .map_or(false, |desc| {
                    desc.contains(drv_name.as_str()) || drv_name.contains(desc.as_str())
                });

            if matches {
                adapter.driver_name = Some(drv_name.clone());
                adapter.driver_version = version.clone();
                adapter.driver_date = date.clone();
            }
        }
    }
}

#[cfg(not(windows))]
pub async fn enrich_driver_info(_adapters: &mut Vec<AdapterInfo>) {
    // No-op on non-Windows — driver info already collected per-platform
}

#[cfg(target_os = "macos")]
async fn collect_adapters_macos() -> Vec<AdapterInfo> {
    let mut adapters = Vec::new();

    if let Ok(output) = tokio::process::Command::new("networksetup")
        .args(["-listallhardwareports"])
        .output()
        .await
    {
        let text = String::from_utf8_lossy(&output.stdout);
        let mut current_name = String::new();

        for line in text.lines() {
            if let Some(name) = line.strip_prefix("Hardware Port: ") {
                current_name = name.trim().to_string();
            } else if let Some(dev) = line.strip_prefix("Device: ") {
                let current_device = dev.trim().to_string();

                let status = if check_interface_up(&current_device).await {
                    "Active"
                } else {
                    "Disconnected"
                };

                adapters.push(AdapterInfo {
                    name: current_name.clone(),
                    adapter_type: detect_macos_type(&current_name),
                    status: status.to_string(),
                    has_ip: status == "Active",
                    description: None,
                    mac_address: None,
                    link_speed_mbps: None,
                    rx_link_speed_mbps: None,
                    dns_servers: None,
                    gateways: None,
                    media_connect_state: None,
                    physical_medium: None,
                    mtu: None,
                    ipv4_metric: None,
                    driver_name: Some(current_device.clone()),
                    driver_version: None,
                    driver_date: None,
                    problem_code: None,
                });
            }
        }
    }

    adapters
}

#[cfg(target_os = "macos")]
fn detect_macos_type(name: &str) -> String {
    let lower = name.to_lowercase();
    if lower.contains("wi-fi") || lower.contains("airport") {
        "Wi-Fi".to_string()
    } else if lower.contains("ethernet") || lower.contains("thunderbolt") {
        "Ethernet".to_string()
    } else if lower.contains("bluetooth") {
        "Bluetooth".to_string()
    } else {
        "Other".to_string()
    }
}

#[cfg(target_os = "macos")]
async fn check_interface_up(device: &str) -> bool {
    if let Ok(output) = tokio::process::Command::new("ifconfig")
        .arg(device)
        .output()
        .await
    {
        let text = String::from_utf8_lossy(&output.stdout);
        text.contains("status: active") || text.contains("inet ")
    } else {
        false
    }
}

#[cfg(target_os = "linux")]
async fn collect_adapters_linux() -> Vec<AdapterInfo> {
    let mut adapters = Vec::new();

    if let Ok(output) = tokio::process::Command::new("ip")
        .args(["-o", "link", "show"])
        .output()
        .await
    {
        let text = String::from_utf8_lossy(&output.stdout);
        for line in text.lines() {
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() < 3 {
                continue;
            }

            let name = parts[1].trim_end_matches(':');
            if name == "lo" {
                continue;
            }

            let is_up = line.contains("state UP") || line.contains(",UP");
            let iface_type = if name.starts_with("wl") {
                "Wi-Fi"
            } else if name.starts_with("en") || name.starts_with("eth") {
                "Ethernet"
            } else if name.starts_with("tun") || name.starts_with("wg") {
                "VPN/Tunnel"
            } else if name.starts_with("docker") || name.starts_with("veth") || name.starts_with("br-") {
                "Virtual"
            } else {
                "Other"
            };

            let driver = get_linux_driver(name).await;

            adapters.push(AdapterInfo {
                name: name.to_string(),
                adapter_type: iface_type.to_string(),
                status: if is_up { "Active" } else { "Disconnected" }.to_string(),
                has_ip: is_up,
                description: None,
                mac_address: None,
                link_speed_mbps: None,
                rx_link_speed_mbps: None,
                dns_servers: None,
                gateways: None,
                media_connect_state: None,
                physical_medium: None,
                mtu: None,
                ipv4_metric: None,
                driver_name: driver,
                driver_version: None,
                driver_date: None,
                problem_code: None,
            });
        }
    }

    adapters
}

#[cfg(target_os = "linux")]
async fn get_linux_driver(iface: &str) -> Option<String> {
    let path = format!("/sys/class/net/{}/device/driver", iface);
    if let Ok(link) = tokio::fs::read_link(&path).await {
        link.file_name()
            .and_then(|n| n.to_str())
            .map(|s| s.to_string())
    } else {
        None
    }
}
