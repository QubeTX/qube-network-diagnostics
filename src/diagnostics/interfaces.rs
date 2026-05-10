use serde::Serialize;
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

pub async fn check() -> (DiagnosticResult, Vec<InterfaceInfo>) {
    let networks = Networks::new_with_refreshed_list();
    let mut details = Vec::new();
    let mut active_count = 0;
    let mut wifi_info = String::new();

    for (name, data) in &networks {
        let mac = format_mac(data.mac_address().0);
        let ip_addrs: Vec<String> = data
            .ip_networks()
            .iter()
            .map(|n| n.addr.to_string())
            .collect();
        let is_up = !ip_addrs.is_empty() && data.total_received() > 0;

        let iface_type = detect_interface_type(name);

        if is_up {
            active_count += 1;
            if iface_type == "Wi-Fi" && wifi_info.is_empty() {
                wifi_info = get_wifi_summary().await;
            }
        }

        details.push(InterfaceInfo {
            name: name.clone(),
            mac,
            ip_addresses: ip_addrs,
            is_up,
            interface_type: iface_type,
            rx_bytes: data.total_received(),
            tx_bytes: data.total_transmitted(),
        });
    }

    let result = if active_count == 0 {
        DiagnosticResult::fail("Network", "No active network interfaces found")
    } else if !wifi_info.is_empty() {
        DiagnosticResult::ok("Network", format!("Connected via {}", wifi_info))
    } else if active_count == 1 {
        let active = details.iter().find(|i| i.is_up);
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
    if lower.contains("wi-fi")
        || lower.contains("wifi")
        || lower.contains("wlan")
        || lower.contains("wlp")
    {
        "Wi-Fi".to_string()
    } else if lower.contains("eth")
        || lower.contains("enp")
        || lower.contains("eno")
        || lower.contains("ethernet")
    {
        "Ethernet".to_string()
    } else if lower == "lo" || lower == "lo0" || (lower.starts_with("lo") && lower.len() <= 3) {
        "Loopback".to_string()
    } else if lower.contains("tun")
        || lower.contains("tap")
        || lower.contains("wg")
        || lower.contains("utun")
    {
        "VPN/Tunnel".to_string()
    } else if lower.contains("bluetooth") || lower.contains("bnep") {
        "Bluetooth".to_string()
    } else if lower.contains("docker") || lower.contains("veth") || lower.contains("br-") {
        "Virtual".to_string()
    } else {
        "Unknown".to_string()
    }
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

async fn get_wifi_summary() -> String {
    #[cfg(windows)]
    {
        match tokio::process::Command::new("netsh")
            .args(["wlan", "show", "interfaces"])
            .output()
            .await
        {
            Ok(output) => {
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
            Err(_) => "Wi-Fi".to_string(),
        }
    }

    #[cfg(target_os = "macos")]
    {
        match tokio::process::Command::new("/System/Library/PrivateFrameworks/Apple80211.framework/Versions/Current/Resources/airport")
            .args(["-I"])
            .output()
            .await
        {
            Ok(output) => {
                let text = String::from_utf8_lossy(&output.stdout);
                let mut ssid = String::new();
                let mut channel = 0u32;
                for line in text.lines() {
                    let line = line.trim();
                    if line.starts_with("SSID:") {
                        ssid = line.split(':').nth(1).map(|s| s.trim().to_string()).unwrap_or_default();
                    }
                    if line.starts_with("channel:") {
                        if let Some(val) = line.split(':').nth(1) {
                            channel = val.trim().split(',').next().and_then(|s| s.parse().ok()).unwrap_or(0);
                        }
                    }
                }
                let band = if channel > 14 && channel <= 177 { "5 GHz" } else if channel <= 14 && channel > 0 { "2.4 GHz" } else { "" };
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
            Err(_) => "Wi-Fi".to_string(),
        }
    }

    #[cfg(target_os = "linux")]
    {
        match tokio::process::Command::new("iwgetid")
            .args(["-r"])
            .output()
            .await
        {
            Ok(output) => {
                let ssid = String::from_utf8_lossy(&output.stdout).trim().to_string();
                // Try to get frequency
                let band = match tokio::process::Command::new("iwgetid")
                    .args(["--freq", "-r"])
                    .output()
                    .await
                {
                    Ok(freq_out) => {
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
                    Err(_) => String::new(),
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
            Err(_) => "Wi-Fi".to_string(),
        }
    }
}
