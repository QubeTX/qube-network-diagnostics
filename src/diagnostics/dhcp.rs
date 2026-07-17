use serde::Serialize;

use super::shared_cache::SharedCache;

#[derive(Debug, Clone, Serialize)]
pub struct DhcpLease {
    pub interface: String,
    pub dhcp_enabled: bool,
    pub dhcp_server: Option<String>,
    pub lease_obtained: Option<String>,
    pub lease_expires: Option<String>,
    pub ip_address: Option<String>,
    pub subnet_mask: Option<String>,
    pub default_gateway: Option<String>,
}

pub async fn collect_with_cache(cache: &SharedCache) -> Option<Vec<DhcpLease>> {
    #[cfg(windows)]
    {
        if let Some(ref ic) = cache.ipconfig {
            return parse_dhcp_from_ipconfig(&ic.raw);
        }
    }
    let _ = cache;
    collect().await
}

pub async fn collect() -> Option<Vec<DhcpLease>> {
    #[cfg(windows)]
    {
        collect_windows().await
    }

    #[cfg(target_os = "macos")]
    {
        collect_macos().await
    }

    #[cfg(target_os = "linux")]
    {
        collect_linux().await
    }
}

#[cfg(windows)]
fn parse_dhcp_from_ipconfig(text: &str) -> Option<Vec<DhcpLease>> {
    let mut leases = Vec::new();
    let mut current = DhcpLease {
        interface: String::new(),
        dhcp_enabled: false,
        dhcp_server: None,
        lease_obtained: None,
        lease_expires: None,
        ip_address: None,
        subnet_mask: None,
        default_gateway: None,
    };
    let mut in_adapter = false;

    for line in text.lines() {
        let line_trimmed = line.trim();

        if !line.starts_with(' ') && !line.starts_with('\t') && line.contains("adapter") {
            if in_adapter && !current.interface.is_empty() {
                leases.push(current.clone());
            }
            current = DhcpLease {
                interface: line_trimmed.trim_end_matches(':').to_string(),
                dhcp_enabled: false,
                dhcp_server: None,
                lease_obtained: None,
                lease_expires: None,
                ip_address: None,
                subnet_mask: None,
                default_gateway: None,
            };
            in_adapter = true;
            continue;
        }

        if !in_adapter {
            continue;
        }

        let get_val =
            |l: &str| -> Option<String> { l.split(':').nth(1).map(|s| s.trim().to_string()) };

        if line_trimmed.contains("DHCP Enabled") {
            current.dhcp_enabled = line_trimmed.contains("Yes");
        } else if line_trimmed.contains("DHCP Server") {
            current.dhcp_server = get_val(line_trimmed);
        } else if line_trimmed.contains("Lease Obtained") {
            current.lease_obtained = get_val(line_trimmed);
        } else if line_trimmed.contains("Lease Expires") {
            current.lease_expires = get_val(line_trimmed);
        } else if line_trimmed.contains("IPv4 Address") || line_trimmed.contains("IP Address") {
            current.ip_address =
                get_val(line_trimmed).map(|s| s.trim_end_matches("(Preferred)").trim().to_string());
        } else if line_trimmed.contains("Subnet Mask") {
            current.subnet_mask = get_val(line_trimmed);
        } else if line_trimmed.contains("Default Gateway") {
            let gw = get_val(line_trimmed);
            if gw.as_ref().map(|s| !s.is_empty()).unwrap_or(false) {
                current.default_gateway = gw;
            }
        }
    }

    if in_adapter && !current.interface.is_empty() {
        leases.push(current);
    }

    let filtered: Vec<DhcpLease> = leases
        .into_iter()
        .filter(|l| l.dhcp_enabled || l.ip_address.is_some())
        .collect();

    Some(filtered)
}

#[cfg(windows)]
async fn collect_windows() -> Option<Vec<DhcpLease>> {
    let mut cmd = tokio::process::Command::new("ipconfig");
    cmd.args(["/all"]);
    let output = super::util::run_with_timeout(cmd, super::util::QUICK).await?;

    let text = String::from_utf8_lossy(&output.stdout);
    parse_dhcp_from_ipconfig(&text)
}

#[cfg(target_os = "macos")]
async fn collect_macos() -> Option<Vec<DhcpLease>> {
    let mut list_cmd = tokio::process::Command::new("/usr/sbin/networksetup");
    list_cmd.arg("-listallhardwareports");
    let interfaces = super::util::run_with_timeout(list_cmd, super::util::QUICK)
        .await
        .map(|output| parse_macos_hardware_devices(&String::from_utf8_lossy(&output.stdout)))
        .filter(|interfaces| !interfaces.is_empty())
        .unwrap_or_else(|| vec!["en0".to_string(), "en1".to_string()]);

    let probes = interfaces.into_iter().map(|iface| async move {
        let mut cmd = tokio::process::Command::new("ipconfig");
        cmd.args(["getpacket", &iface]);
        if let Some(output) = super::util::run_with_timeout(cmd, super::util::QUICK).await {
            if output.status.success() {
                let text = String::from_utf8_lossy(&output.stdout);
                return parse_macos_dhcp_packet(&iface, &text);
            }
        }
        None
    });
    let leases: Vec<DhcpLease> = futures_util::future::join_all(probes)
        .await
        .into_iter()
        .flatten()
        .collect();

    if leases.is_empty() {
        None
    } else {
        Some(leases)
    }
}

#[cfg(any(target_os = "macos", test))]
fn parse_macos_hardware_devices(text: &str) -> Vec<String> {
    text.lines()
        .filter_map(|line| line.trim().strip_prefix("Device:"))
        .map(str::trim)
        .filter(|device| !device.is_empty())
        .map(str::to_string)
        .collect()
}

#[cfg(any(target_os = "macos", test))]
fn normalize_macos_packet_value(value: &str) -> Option<String> {
    let value = value
        .trim()
        .trim_start_matches('{')
        .trim_end_matches('}')
        .trim();
    (!value.is_empty()).then(|| value.to_string())
}

#[cfg(any(target_os = "macos", test))]
fn parse_macos_u32(value: &str) -> Option<u32> {
    let value = value.trim();
    value
        .strip_prefix("0x")
        .and_then(|hex| u32::from_str_radix(hex, 16).ok())
        .or_else(|| value.parse().ok())
}

#[cfg(any(target_os = "macos", test))]
fn parse_macos_dhcp_packet(interface: &str, text: &str) -> Option<DhcpLease> {
    let mut lease = DhcpLease {
        interface: interface.to_string(),
        dhcp_enabled: true,
        dhcp_server: None,
        lease_obtained: None,
        lease_expires: None,
        ip_address: None,
        subnet_mask: None,
        default_gateway: None,
    };
    for line in text.lines().map(str::trim) {
        if let Some(value) = line.strip_prefix("yiaddr =") {
            lease.ip_address = normalize_macos_packet_value(value);
        } else if line.starts_with("server_identifier") {
            lease.dhcp_server = line
                .split_once(':')
                .and_then(|(_, value)| normalize_macos_packet_value(value));
        } else if line.starts_with("subnet_mask") {
            lease.subnet_mask = line
                .split_once(':')
                .and_then(|(_, value)| normalize_macos_packet_value(value));
        } else if line.starts_with("router") {
            lease.default_gateway = line
                .split_once(':')
                .and_then(|(_, value)| normalize_macos_packet_value(value));
        } else if line.starts_with("lease_time") {
            // DHCP option 51 is a duration measured from lease acquisition,
            // not an expiry timestamp relative to when this diagnostic runs.
            // Keep the existing serialized field while labeling the value
            // honestly instead of claiming it expires "from now".
            lease.lease_expires = line
                .split_once(':')
                .and_then(|(_, value)| parse_macos_u32(value))
                .map(|seconds| format!("{seconds}s lease duration"));
        }
    }
    lease.ip_address.as_ref()?;
    Some(lease)
}

#[cfg(target_os = "linux")]
async fn collect_linux() -> Option<Vec<DhcpLease>> {
    let mut leases = Vec::new();

    // Check common DHCP lease file locations (exact paths)
    let exact_paths = [
        "/var/lib/dhcp/dhclient.leases",
        "/var/lib/dhclient/dhclient.leases",
    ];

    for path in &exact_paths {
        if let Ok(content) = tokio::fs::read_to_string(path).await {
            if let Some(lease) = parse_last_dhclient_lease(&content) {
                leases.push(lease);
            }
        }
    }

    // Scan NetworkManager directory for dhclient-*.leases files
    if let Ok(mut dir) = tokio::fs::read_dir("/var/lib/NetworkManager").await {
        while let Ok(Some(entry)) = dir.next_entry().await {
            let name = entry.file_name().to_string_lossy().to_string();
            if name.starts_with("dhclient-") && name.ends_with(".leases") {
                if let Ok(content) = tokio::fs::read_to_string(entry.path()).await {
                    if let Some(lease) = parse_last_dhclient_lease(&content) {
                        leases.push(lease);
                    }
                }
            }
        }
    }

    if leases.is_empty() {
        None
    } else {
        Some(leases)
    }
}

#[cfg(target_os = "linux")]
fn parse_last_dhclient_lease(content: &str) -> Option<DhcpLease> {
    // Split on "lease {" boundaries and parse only the last block
    let blocks: Vec<&str> = content.split("lease {").collect();
    let last_block = blocks.last().filter(|b| b.contains("}"))?;

    let mut lease = DhcpLease {
        interface: "default".to_string(),
        dhcp_enabled: true,
        dhcp_server: None,
        lease_obtained: None,
        lease_expires: None,
        ip_address: None,
        subnet_mask: None,
        default_gateway: None,
    };

    for line in last_block.lines() {
        let line = line.trim().trim_end_matches(';');
        if let Some(val) = line.strip_prefix("fixed-address ") {
            lease.ip_address = Some(val.to_string());
        } else if let Some(val) = line.strip_prefix("option dhcp-server-identifier ") {
            lease.dhcp_server = Some(val.to_string());
        } else if let Some(val) = line.strip_prefix("option subnet-mask ") {
            lease.subnet_mask = Some(val.to_string());
        } else if let Some(val) = line.strip_prefix("option routers ") {
            lease.default_gateway = Some(val.to_string());
        } else if let Some(val) = line.strip_prefix("renew ") {
            lease.lease_obtained = Some(val.to_string());
        } else if let Some(val) = line.strip_prefix("expire ") {
            lease.lease_expires = Some(val.to_string());
        }
    }

    if lease.ip_address.is_some() {
        Some(lease)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn macos_discovers_all_hardware_devices() {
        let text =
            "Hardware Port: Wi-Fi\nDevice: en0\n\nHardware Port: Ethernet Adapter\nDevice: en3\n";
        assert_eq!(parse_macos_hardware_devices(text), vec!["en0", "en3"]);
    }

    #[test]
    fn macos_packet_normalizes_typed_hex_and_braced_values() {
        let text = "yiaddr = 10.1.0.91\nserver_identifier (ip): 10.1.0.1\nlease_time (uint32): 0xa8c0\nsubnet_mask (ip): 255.255.255.0\nrouter (ip_mult): {10.1.0.1}\n";
        let lease = parse_macos_dhcp_packet("en0", text).unwrap();
        assert_eq!(lease.ip_address.as_deref(), Some("10.1.0.91"));
        assert_eq!(lease.dhcp_server.as_deref(), Some("10.1.0.1"));
        assert_eq!(lease.default_gateway.as_deref(), Some("10.1.0.1"));
        assert_eq!(
            lease.lease_expires.as_deref(),
            Some("43200s lease duration")
        );
    }
}
