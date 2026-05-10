use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct MtuInfo {
    pub interface: String,
    pub mtu: u32,
}

pub async fn collect() -> Option<Vec<MtuInfo>> {
    let mut results = Vec::new();

    #[cfg(windows)]
    {
        if let Ok(output) = tokio::process::Command::new("netsh")
            .args(["interface", "ipv4", "show", "subinterfaces"])
            .output()
            .await
        {
            let text = String::from_utf8_lossy(&output.stdout);
            for line in text.lines() {
                let parts: Vec<&str> = line.split_whitespace().collect();
                // Format: MTU  MediaSenseState   Bytes In  Bytes Out  Interface
                if parts.len() >= 5 {
                    if let Ok(mtu) = parts[0].parse::<u32>() {
                        let iface = parts[4..].join(" ");
                        results.push(MtuInfo {
                            interface: iface,
                            mtu,
                        });
                    }
                }
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
                if line.contains("mtu") {
                    let parts: Vec<&str> = line.split_whitespace().collect();
                    if let Some(name_idx) = parts
                        .iter()
                        .position(|p| p.ends_with(':') && !p.starts_with(' '))
                    {
                        let name = parts[name_idx].trim_end_matches(':');
                        if let Some(mtu_idx) = parts.iter().position(|p| *p == "mtu") {
                            if let Some(mtu_val) = parts.get(mtu_idx + 1) {
                                if let Ok(mtu) = mtu_val.parse::<u32>() {
                                    results.push(MtuInfo {
                                        interface: name.to_string(),
                                        mtu,
                                    });
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    #[cfg(target_os = "macos")]
    {
        if let Ok(output) = tokio::process::Command::new("ifconfig").output().await {
            let text = String::from_utf8_lossy(&output.stdout);
            let mut current_iface = String::new();

            for line in text.lines() {
                if !line.starts_with('\t') && !line.starts_with(' ') {
                    current_iface = line.split(':').next().unwrap_or("").to_string();
                }
                if line.contains("mtu") {
                    let parts: Vec<&str> = line.split_whitespace().collect();
                    if let Some(mtu_idx) = parts.iter().position(|p| *p == "mtu") {
                        if let Some(mtu_val) = parts.get(mtu_idx + 1) {
                            if let Ok(mtu) = mtu_val.parse::<u32>() {
                                results.push(MtuInfo {
                                    interface: current_iface.clone(),
                                    mtu,
                                });
                            }
                        }
                    }
                }
            }
        }
    }

    if results.is_empty() {
        None
    } else {
        Some(results)
    }
}
