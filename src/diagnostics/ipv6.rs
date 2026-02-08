use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct Ipv6Info {
    pub available: bool,
    pub addresses: Vec<Ipv6Address>,
    pub connectivity: Ipv6Connectivity,
    pub dual_stack: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct Ipv6Address {
    pub interface: String,
    pub address: String,
    pub scope: String,
}

#[derive(Debug, Clone, Serialize)]
pub enum Ipv6Connectivity {
    Full,
    LinkLocal,
    None,
}

pub async fn collect() -> Option<Ipv6Info> {
    let addresses = get_ipv6_addresses().await;
    let has_global = addresses.iter().any(|a| a.scope == "global");
    let has_link_local = addresses.iter().any(|a| a.scope == "link-local");

    // Test IPv6 connectivity
    let connectivity = if has_global {
        test_ipv6_connectivity().await
    } else if has_link_local {
        Ipv6Connectivity::LinkLocal
    } else {
        Ipv6Connectivity::None
    };

    let dual_stack = has_global;

    Some(Ipv6Info {
        available: !addresses.is_empty(),
        addresses,
        connectivity,
        dual_stack,
    })
}

async fn get_ipv6_addresses() -> Vec<Ipv6Address> {
    let mut addrs = Vec::new();

    #[cfg(windows)]
    {
        if let Ok(output) = tokio::process::Command::new("ipconfig")
            .output()
            .await
        {
            let text = String::from_utf8_lossy(&output.stdout);
            let mut current_iface = String::new();

            for line in text.lines() {
                if !line.starts_with(' ') && !line.starts_with('\t') && line.contains("adapter") {
                    current_iface = line.trim().trim_end_matches(':').to_string();
                }

                let trimmed = line.trim();
                if trimmed.contains("IPv6 Address") || trimmed.contains("Link-local IPv6") || trimmed.contains("Temporary IPv6") {
                    if let Some(addr) = trimmed.split(':').skip(1).collect::<Vec<&str>>().join(":").trim().strip_suffix("(Preferred)") {
                        let scope = if trimmed.contains("Link-local") { "link-local" } else { "global" };
                        addrs.push(Ipv6Address {
                            interface: current_iface.clone(),
                            address: addr.trim().to_string(),
                            scope: scope.to_string(),
                        });
                    } else {
                        let addr: String = trimmed.split(':').skip(1).collect::<Vec<&str>>().join(":");
                        let addr = addr.trim().trim_end_matches("(Preferred)").trim();
                        if !addr.is_empty() {
                            let scope = if trimmed.contains("Link-local") { "link-local" } else { "global" };
                            addrs.push(Ipv6Address {
                                interface: current_iface.clone(),
                                address: addr.to_string(),
                                scope: scope.to_string(),
                            });
                        }
                    }
                }
            }
        }
    }

    #[cfg(unix)]
    {
        if let Ok(output) = tokio::process::Command::new("ip")
            .args(["-6", "addr", "show"])
            .output()
            .await
        {
            let text = String::from_utf8_lossy(&output.stdout);
            let mut current_iface = String::new();

            for line in text.lines() {
                let trimmed = line.trim();
                if !line.starts_with(' ') {
                    current_iface = trimmed.split(':').nth(1).unwrap_or("").trim().to_string();
                } else if trimmed.starts_with("inet6") {
                    let parts: Vec<&str> = trimmed.split_whitespace().collect();
                    if parts.len() >= 4 {
                        let addr = parts[1].split('/').next().unwrap_or(parts[1]);
                        let scope = parts.get(3).unwrap_or(&"unknown").to_string();
                        addrs.push(Ipv6Address {
                            interface: current_iface.clone(),
                            address: addr.to_string(),
                            scope,
                        });
                    }
                }
            }
        }

        // Fallback for macOS if `ip` not available
        if addrs.is_empty() {
            if let Ok(output) = tokio::process::Command::new("ifconfig")
                .output()
                .await
            {
                let text = String::from_utf8_lossy(&output.stdout);
                let mut current_iface = String::new();

                for line in text.lines() {
                    if !line.starts_with('\t') && !line.starts_with(' ') {
                        current_iface = line.split(':').next().unwrap_or("").to_string();
                    } else if line.contains("inet6") {
                        let parts: Vec<&str> = line.split_whitespace().collect();
                        if let Some(addr) = parts.get(1) {
                            let scope = if addr.starts_with("fe80") {
                                "link-local"
                            } else if addr == "::1" {
                                "loopback"
                            } else {
                                "global"
                            };
                            addrs.push(Ipv6Address {
                                interface: current_iface.clone(),
                                address: addr.to_string(),
                                scope: scope.to_string(),
                            });
                        }
                    }
                }
            }
        }
    }

    addrs
}

async fn test_ipv6_connectivity() -> Ipv6Connectivity {
    // Try to connect to Google's IPv6 DNS
    match tokio::time::timeout(
        std::time::Duration::from_secs(5),
        tokio::net::TcpStream::connect("[2001:4860:4860::8888]:443"),
    )
    .await
    {
        Ok(Ok(_)) => Ipv6Connectivity::Full,
        _ => {
            // Try Cloudflare IPv6
            match tokio::time::timeout(
                std::time::Duration::from_secs(3),
                tokio::net::TcpStream::connect("[2606:4700:4700::1111]:443"),
            )
            .await
            {
                Ok(Ok(_)) => Ipv6Connectivity::Full,
                _ => Ipv6Connectivity::None,
            }
        }
    }
}
