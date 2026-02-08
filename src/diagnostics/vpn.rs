use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct VpnAdapter {
    pub name: String,
    pub adapter_type: String,
    pub status: String,
    pub ip_address: Option<String>,
}

pub async fn collect() -> Option<Vec<VpnAdapter>> {
    let mut vpns = Vec::new();

    #[cfg(windows)]
    {
        // Check for VPN adapters via ipconfig
        if let Ok(output) = tokio::process::Command::new("ipconfig")
            .args(["/all"])
            .output()
            .await
        {
            let text = String::from_utf8_lossy(&output.stdout);
            let mut current_name = String::new();
            let mut current_ip = None;

            for line in text.lines() {
                if !line.starts_with(' ') && !line.starts_with('\t') && line.contains("adapter") {
                    let name = line.trim().trim_end_matches(':');
                    let lower = name.to_lowercase();
                    if lower.contains("vpn") || lower.contains("tap") || lower.contains("tun")
                        || lower.contains("wireguard") || lower.contains("wintun")
                        || lower.contains("fortinet") || lower.contains("cisco")
                        || lower.contains("palo alto") || lower.contains("global protect")
                    {
                        if !current_name.is_empty() {
                            vpns.push(VpnAdapter {
                                name: current_name.clone(),
                                adapter_type: detect_vpn_type(&current_name),
                                status: if current_ip.is_some() { "Connected" } else { "Disconnected" }.to_string(),
                                ip_address: current_ip.take(),
                            });
                        }
                        current_name = name.to_string();
                        current_ip = None;
                    } else {
                        current_name.clear();
                    }
                } else if !current_name.is_empty() {
                    let trimmed = line.trim();
                    if trimmed.contains("IPv4 Address") || (trimmed.contains("IP Address") && !trimmed.contains("Autoconfiguration")) {
                        current_ip = trimmed.split(':').nth(1).map(|s| {
                            s.trim().trim_end_matches("(Preferred)").trim().to_string()
                        });
                    }
                }
            }

            if !current_name.is_empty() {
                vpns.push(VpnAdapter {
                    name: current_name.clone(),
                    adapter_type: detect_vpn_type(&current_name),
                    status: if current_ip.is_some() { "Connected" } else { "Disconnected" }.to_string(),
                    ip_address: current_ip,
                });
            }
        }
    }

    #[cfg(target_os = "macos")]
    {
        if let Ok(output) = tokio::process::Command::new("ifconfig")
            .output()
            .await
        {
            let text = String::from_utf8_lossy(&output.stdout);
            let mut current_iface = String::new();
            let mut current_ip = None;

            for line in text.lines() {
                if !line.starts_with('\t') && !line.starts_with(' ') {
                    if !current_iface.is_empty() && is_vpn_interface(&current_iface) {
                        vpns.push(VpnAdapter {
                            name: current_iface.clone(),
                            adapter_type: detect_vpn_type(&current_iface),
                            status: if current_ip.is_some() { "Connected" } else { "Disconnected" }.to_string(),
                            ip_address: current_ip.take(),
                        });
                    }
                    current_iface = line.split(':').next().unwrap_or("").to_string();
                    current_ip = None;
                } else if line.contains("inet ") {
                    current_ip = line.split_whitespace().nth(1).map(|s| s.to_string());
                }
            }

            if !current_iface.is_empty() && is_vpn_interface(&current_iface) {
                vpns.push(VpnAdapter {
                    name: current_iface.clone(),
                    adapter_type: detect_vpn_type(&current_iface),
                    status: if current_ip.is_some() { "Connected" } else { "Disconnected" }.to_string(),
                    ip_address: current_ip,
                });
            }
        }
    }

    #[cfg(target_os = "linux")]
    {
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
                        vpns.push(VpnAdapter {
                            name: name.to_string(),
                            adapter_type: detect_vpn_type(name),
                            status: if is_up { "Connected" } else { "Disconnected" }.to_string(),
                            ip_address: None,
                        });
                    }
                }
            }
        }
    }

    if vpns.is_empty() {
        None
    } else {
        Some(vpns)
    }
}

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
    } else if lower.contains("cisco") {
        "Cisco AnyConnect".to_string()
    } else if lower.contains("fortinet") || lower.contains("forticlient") {
        "FortiClient".to_string()
    } else if lower.contains("global protect") || lower.contains("palo alto") {
        "GlobalProtect".to_string()
    } else {
        "VPN".to_string()
    }
}
