use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct ArpEntry {
    pub ip: String,
    pub mac: String,
    pub interface: String,
    pub entry_type: String,
}

// ── ARP gateway health (v3.4.0+) ────────────────────────────────────────────

#[derive(Debug, Clone, Serialize)]
pub struct DuplicateIpMacs {
    pub ip: String,
    pub macs: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ArpHealth {
    pub gateway_in_table: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gateway_mac: Option<String>,
    /// IPs that appear with more than one distinct MAC — an ARP-spoofing or
    /// dual-router indicator.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub duplicate_ip_macs: Vec<DuplicateIpMacs>,
    pub assessment: String,
    pub level: String,
}

/// Pure post-processing over an already-collected ARP table — zero extra
/// runtime. `None` when there's nothing to assess.
pub fn assess_health(
    arp_table: Option<&[ArpEntry]>,
    gateway_ip: Option<&str>,
) -> Option<ArpHealth> {
    let table = arp_table?;
    if table.is_empty() {
        return None;
    }

    let gateway_entry = gateway_ip.and_then(|gw| table.iter().find(|e| e.ip == gw));
    let gateway_in_table = gateway_entry.is_some();
    let gateway_mac = gateway_entry.map(|e| e.mac.clone());

    // Group MACs per IP; flag IPs with more than one distinct MAC. Ignore
    // broadcast/multicast pseudo-entries.
    let mut per_ip: std::collections::HashMap<&str, Vec<&str>> = std::collections::HashMap::new();
    for entry in table {
        if entry.mac.eq_ignore_ascii_case("ff-ff-ff-ff-ff-ff")
            || entry.mac.eq_ignore_ascii_case("ff:ff:ff:ff:ff:ff")
        {
            continue;
        }
        let macs = per_ip.entry(entry.ip.as_str()).or_default();
        if !macs
            .iter()
            .any(|m| m.eq_ignore_ascii_case(entry.mac.as_str()))
        {
            macs.push(entry.mac.as_str());
        }
    }
    let mut duplicate_ip_macs: Vec<DuplicateIpMacs> = per_ip
        .into_iter()
        .filter(|(_, macs)| macs.len() > 1)
        .map(|(ip, macs)| DuplicateIpMacs {
            ip: ip.to_string(),
            macs: macs.into_iter().map(|m| m.to_string()).collect(),
        })
        .collect();
    duplicate_ip_macs.sort_by(|a, b| a.ip.cmp(&b.ip));

    let (assessment, level) = if !duplicate_ip_macs.is_empty() {
        (
            "An IP maps to multiple MAC addresses — possible ARP spoofing or a misconfigured second router",
            "warn",
        )
    } else if gateway_ip.is_some() && !gateway_in_table {
        (
            "Gateway is missing from the ARP table after a full diagnostic run — possible L2 problem",
            "warn",
        )
    } else {
        ("ARP table looks healthy", "ok")
    };

    Some(ArpHealth {
        gateway_in_table,
        gateway_mac,
        duplicate_ip_macs,
        assessment: assessment.to_string(),
        level: level.to_string(),
    })
}

pub async fn collect() -> Option<Vec<ArpEntry>> {
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
async fn collect_windows() -> Option<Vec<ArpEntry>> {
    let mut cmd = tokio::process::Command::new("arp");
    cmd.args(["-a"]);
    let output = super::util::run_with_timeout(cmd, super::util::QUICK).await?;

    let text = String::from_utf8_lossy(&output.stdout);
    let mut entries = Vec::new();
    let mut current_iface = String::new();

    for line in text.lines() {
        let line = line.trim();
        if line.starts_with("Interface:") {
            current_iface = line
                .split_whitespace()
                .nth(1)
                .unwrap_or("unknown")
                .to_string();
        } else if !line.is_empty() && !line.starts_with("Internet") {
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() >= 3 {
                entries.push(ArpEntry {
                    ip: parts[0].to_string(),
                    mac: parts[1].to_string(),
                    interface: current_iface.clone(),
                    entry_type: parts[2].to_string(),
                });
            }
        }
    }

    Some(entries)
}

#[cfg(target_os = "macos")]
async fn collect_macos() -> Option<Vec<ArpEntry>> {
    let mut cmd = tokio::process::Command::new("arp");
    cmd.args(["-a"]);
    let output = super::util::run_with_timeout(cmd, super::util::QUICK).await?;

    let text = String::from_utf8_lossy(&output.stdout);
    let mut entries = Vec::new();

    for line in text.lines() {
        // Format: ? (192.168.1.1) at aa:bb:cc:dd:ee:ff on en0 ifscope [ethernet]
        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.len() >= 6 && parts[1].starts_with('(') {
            let ip = parts[1].trim_matches(|c| c == '(' || c == ')').to_string();
            let mac = parts[3].to_string();
            let iface = parts.get(5).unwrap_or(&"unknown").to_string();

            entries.push(ArpEntry {
                ip,
                mac,
                interface: iface,
                entry_type: "dynamic".to_string(),
            });
        }
    }

    Some(entries)
}

#[cfg(target_os = "linux")]
async fn collect_linux() -> Option<Vec<ArpEntry>> {
    // Try /proc/net/arp first
    if let Ok(content) = tokio::fs::read_to_string("/proc/net/arp").await {
        let mut entries = Vec::new();
        for line in content.lines().skip(1) {
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() >= 6 {
                entries.push(ArpEntry {
                    ip: parts[0].to_string(),
                    mac: parts[3].to_string(),
                    interface: parts[5].to_string(),
                    entry_type: if parts[2] == "0x2" {
                        "dynamic".to_string()
                    } else {
                        "static".to_string()
                    },
                });
            }
        }
        return Some(entries);
    }

    None
}

#[cfg(test)]
mod health_tests {
    use super::*;

    fn entry(ip: &str, mac: &str) -> ArpEntry {
        ArpEntry {
            ip: ip.to_string(),
            mac: mac.to_string(),
            interface: "eth0".to_string(),
            entry_type: "dynamic".to_string(),
        }
    }

    #[test]
    fn healthy_table_with_gateway() {
        let table = [
            entry("192.168.1.1", "aa-bb-cc-dd-ee-ff"),
            entry("192.168.1.50", "11-22-33-44-55-66"),
        ];
        let h = assess_health(Some(&table), Some("192.168.1.1")).unwrap();
        assert!(h.gateway_in_table);
        assert_eq!(h.gateway_mac.as_deref(), Some("aa-bb-cc-dd-ee-ff"));
        assert_eq!(h.level, "ok");
    }

    #[test]
    fn duplicate_macs_flagged() {
        let table = [
            entry("192.168.1.1", "aa-bb-cc-dd-ee-ff"),
            entry("192.168.1.1", "11-22-33-44-55-66"),
        ];
        let h = assess_health(Some(&table), Some("192.168.1.1")).unwrap();
        assert_eq!(h.duplicate_ip_macs.len(), 1);
        assert_eq!(h.level, "warn");
        assert!(h.assessment.contains("spoofing"));
    }

    #[test]
    fn missing_gateway_flagged() {
        let table = [entry("192.168.1.50", "11-22-33-44-55-66")];
        let h = assess_health(Some(&table), Some("192.168.1.1")).unwrap();
        assert!(!h.gateway_in_table);
        assert_eq!(h.level, "warn");
    }

    #[test]
    fn broadcast_entries_ignored() {
        let table = [
            entry("192.168.1.255", "ff-ff-ff-ff-ff-ff"),
            entry("192.168.1.255", "FF:FF:FF:FF:FF:FF"),
        ];
        let h = assess_health(Some(&table), None).unwrap();
        assert!(h.duplicate_ip_macs.is_empty());
    }

    #[test]
    fn empty_or_missing_table_is_none() {
        assert!(assess_health(None, Some("192.168.1.1")).is_none());
        assert!(assess_health(Some(&[]), Some("192.168.1.1")).is_none());
    }
}
