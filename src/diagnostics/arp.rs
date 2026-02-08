use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct ArpEntry {
    pub ip: String,
    pub mac: String,
    pub interface: String,
    pub entry_type: String,
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
    let output = tokio::process::Command::new("arp")
        .args(["-a"])
        .output()
        .await
        .ok()?;

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
    let output = tokio::process::Command::new("arp")
        .args(["-a"])
        .output()
        .await
        .ok()?;

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
