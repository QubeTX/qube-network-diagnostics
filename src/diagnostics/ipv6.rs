use serde::Serialize;

use super::shared_cache::SharedCache;

#[derive(Debug, Clone, Serialize)]
pub struct Ipv6Info {
    pub available: bool,
    pub addresses: Vec<Ipv6Address>,
    pub connectivity: Ipv6Connectivity,
    pub dual_stack: bool,
    /// Real HTTPS-over-v6 fetch succeeded (additive, v3.4.0+).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub v6_http_ok: Option<bool>,
    /// Timed TCP connect to a v4 anycast endpoint (ms).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub v4_connect_ms: Option<f64>,
    /// Timed TCP connect to a v6 anycast endpoint (ms).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub v6_connect_ms: Option<f64>,
    /// v6 − v4 connect time: the happy-eyeballs penalty users feel when v6
    /// is configured but slow.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub v6_penalty_ms: Option<f64>,
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

pub async fn collect_with_cache(cache: &SharedCache) -> Option<Ipv6Info> {
    let addresses = get_ipv6_addresses_cached(cache).await;
    let has_global = addresses.iter().any(|a| a.scope == "global");
    let has_link_local = addresses.iter().any(|a| a.scope == "link-local");

    let connectivity = if has_global {
        test_ipv6_connectivity().await
    } else if has_link_local {
        Ipv6Connectivity::LinkLocal
    } else {
        Ipv6Connectivity::None
    };

    let dual_stack = has_global;

    // Deep probes (v3.4.0+): only meaningful when a global v6 address exists.
    let (v6_http_ok, v4_connect_ms, v6_connect_ms, v6_penalty_ms) = if has_global {
        let v6_http = fetch_over_v6().await;
        let v4_ms = timed_connect("1.1.1.1:443").await;
        let v6_ms = timed_connect("[2606:4700:4700::1111]:443").await;
        let penalty = match (v4_ms, v6_ms) {
            (Some(v4), Some(v6)) => Some(v6 - v4),
            _ => None,
        };
        (Some(v6_http), v4_ms, v6_ms, penalty)
    } else {
        (None, None, None, None)
    };

    Some(Ipv6Info {
        available: !addresses.is_empty(),
        addresses,
        connectivity,
        dual_stack,
        v6_http_ok,
        v4_connect_ms,
        v6_connect_ms,
        v6_penalty_ms,
    })
}

/// HTTPS fetch pinned to an IPv6 literal — proves real end-to-end v6, not
/// just an address assignment.
async fn fetch_over_v6() -> bool {
    let Ok(client) = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(5))
        .build()
    else {
        return false;
    };
    client
        .get("https://[2606:4700:4700::1111]/cdn-cgi/trace")
        .send()
        .await
        .map(|r| r.status().is_success())
        .unwrap_or(false)
}

/// Timed TCP connect — used for the v4-vs-v6 happy-eyeballs comparison.
async fn timed_connect(addr: &str) -> Option<f64> {
    let start = std::time::Instant::now();
    match tokio::time::timeout(
        std::time::Duration::from_secs(5),
        tokio::net::TcpStream::connect(addr),
    )
    .await
    {
        Ok(Ok(_)) => Some(start.elapsed().as_secs_f64() * 1000.0),
        _ => None,
    }
}

async fn get_ipv6_addresses_cached(cache: &SharedCache) -> Vec<Ipv6Address> {
    #[cfg(windows)]
    {
        // ipconfig /all is a superset of plain ipconfig — same IPv6 fields
        if let Some(ref ic) = cache.ipconfig {
            return parse_ipv6_from_ipconfig(&ic.raw);
        }
    }
    let _ = cache;
    get_ipv6_addresses().await
}

#[cfg(windows)]
fn parse_ipv6_from_ipconfig(text: &str) -> Vec<Ipv6Address> {
    let mut addrs = Vec::new();
    let mut current_iface = String::new();

    for line in text.lines() {
        if !line.starts_with(' ') && !line.starts_with('\t') && line.contains("adapter") {
            current_iface = line.trim().trim_end_matches(':').to_string();
        }

        let trimmed = line.trim();
        if trimmed.contains("IPv6 Address")
            || trimmed.contains("Link-local IPv6")
            || trimmed.contains("Temporary IPv6")
        {
            if let Some(addr) = trimmed
                .split(':')
                .skip(1)
                .collect::<Vec<&str>>()
                .join(":")
                .trim()
                .strip_suffix("(Preferred)")
            {
                let scope = if trimmed.contains("Link-local") {
                    "link-local"
                } else {
                    "global"
                };
                addrs.push(Ipv6Address {
                    interface: current_iface.clone(),
                    address: addr.trim().to_string(),
                    scope: scope.to_string(),
                });
            } else {
                let addr: String = trimmed.split(':').skip(1).collect::<Vec<&str>>().join(":");
                let addr = addr.trim().trim_end_matches("(Preferred)").trim();
                if !addr.is_empty() {
                    let scope = if trimmed.contains("Link-local") {
                        "link-local"
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

    addrs
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
        // The cache-less path skips the deep probes (used outside tech mode).
        v6_http_ok: None,
        v4_connect_ms: None,
        v6_connect_ms: None,
        v6_penalty_ms: None,
    })
}

async fn get_ipv6_addresses() -> Vec<Ipv6Address> {
    let mut addrs = Vec::new();

    #[cfg(windows)]
    {
        let cmd = tokio::process::Command::new("ipconfig");
        if let Some(output) = super::util::run_with_timeout(cmd, super::util::QUICK).await {
            let text = String::from_utf8_lossy(&output.stdout);
            let mut current_iface = String::new();

            for line in text.lines() {
                if !line.starts_with(' ') && !line.starts_with('\t') && line.contains("adapter") {
                    current_iface = line.trim().trim_end_matches(':').to_string();
                }

                let trimmed = line.trim();
                if trimmed.contains("IPv6 Address")
                    || trimmed.contains("Link-local IPv6")
                    || trimmed.contains("Temporary IPv6")
                {
                    if let Some(addr) = trimmed
                        .split(':')
                        .skip(1)
                        .collect::<Vec<&str>>()
                        .join(":")
                        .trim()
                        .strip_suffix("(Preferred)")
                    {
                        let scope = if trimmed.contains("Link-local") {
                            "link-local"
                        } else {
                            "global"
                        };
                        addrs.push(Ipv6Address {
                            interface: current_iface.clone(),
                            address: addr.trim().to_string(),
                            scope: scope.to_string(),
                        });
                    } else {
                        let addr: String =
                            trimmed.split(':').skip(1).collect::<Vec<&str>>().join(":");
                        let addr = addr.trim().trim_end_matches("(Preferred)").trim();
                        if !addr.is_empty() {
                            let scope = if trimmed.contains("Link-local") {
                                "link-local"
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

    #[cfg(unix)]
    {
        let mut cmd = tokio::process::Command::new("ip");
        cmd.args(["-6", "addr", "show"]);
        if let Some(output) = super::util::run_with_timeout(cmd, super::util::QUICK).await {
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
            let cmd = tokio::process::Command::new("ifconfig");
            if let Some(output) = super::util::run_with_timeout(cmd, super::util::QUICK).await {
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
                            } else if *addr == "::1" {
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
