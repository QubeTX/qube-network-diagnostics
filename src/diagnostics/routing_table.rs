use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct RouteEntry {
    pub destination: String,
    pub gateway: String,
    pub mask: String,
    pub interface: String,
    pub metric: Option<u32>,
    pub flags: Option<String>,
}

pub async fn collect() -> Option<Vec<RouteEntry>> {
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
async fn collect_windows() -> Option<Vec<RouteEntry>> {
    let mut cmd = tokio::process::Command::new("route");
    cmd.args(["print", "-4"]);
    let output = super::util::run_with_timeout(cmd, super::util::QUICK).await?;

    let text = String::from_utf8_lossy(&output.stdout);
    let mut entries = Vec::new();
    let mut in_routes = false;

    for line in text.lines() {
        let line = line.trim();
        if line.starts_with("Network Destination") {
            in_routes = true;
            continue;
        }
        if line.starts_with("=") || line.is_empty() {
            if in_routes && !entries.is_empty() {
                break;
            }
            continue;
        }

        if in_routes {
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() >= 4 {
                entries.push(RouteEntry {
                    destination: parts[0].to_string(),
                    mask: parts[1].to_string(),
                    gateway: parts[2].to_string(),
                    interface: parts[3].to_string(),
                    metric: parts.get(4).and_then(|s| s.parse().ok()),
                    flags: None,
                });
            }
        }
    }

    Some(entries)
}

#[cfg(target_os = "macos")]
async fn collect_macos() -> Option<Vec<RouteEntry>> {
    let mut cmd = tokio::process::Command::new("netstat");
    cmd.args(["-rn", "-f", "inet"]);
    let output = super::util::run_with_timeout(cmd, super::util::QUICK).await?;

    let text = String::from_utf8_lossy(&output.stdout);
    Some(parse_macos_routes(&text))
}

#[cfg(any(target_os = "macos", test))]
fn parse_macos_routes(text: &str) -> Vec<RouteEntry> {
    let mut entries = Vec::new();
    for line in text.lines() {
        if line.starts_with("Destination")
            || line.starts_with("Routing")
            || line.starts_with("Internet")
        {
            continue;
        }
        let parts: Vec<&str> = line.split_whitespace().collect();
        // Darwin's columns are Destination Gateway Flags Netif [Expire].
        // `Expire` is optional and often `!`; taking the final field therefore
        // produced bogus interface names in technician output.
        if parts.len() >= 4 && !parts[0].ends_with(':') {
            entries.push(RouteEntry {
                destination: parts[0].to_string(),
                gateway: parts[1].to_string(),
                mask: String::new(),
                flags: Some(parts[2].to_string()),
                interface: parts[3].to_string(),
                metric: None,
            });
        }
    }

    entries
}

#[cfg(target_os = "linux")]
async fn collect_linux() -> Option<Vec<RouteEntry>> {
    let mut cmd = tokio::process::Command::new("ip");
    cmd.args(["route", "show"]);
    let output = super::util::run_with_timeout(cmd, super::util::QUICK).await?;

    let text = String::from_utf8_lossy(&output.stdout);
    let mut entries = Vec::new();

    for line in text.lines() {
        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.is_empty() {
            continue;
        }

        let dest = parts[0].to_string();
        let mut gateway = String::new();
        let mut iface = String::new();
        let mut metric = None;

        let mut i = 1;
        while i < parts.len() {
            match parts[i] {
                "via" if i + 1 < parts.len() => {
                    gateway = parts[i + 1].to_string();
                    i += 1;
                }
                "dev" if i + 1 < parts.len() => {
                    iface = parts[i + 1].to_string();
                    i += 1;
                }
                "metric" if i + 1 < parts.len() => {
                    metric = parts[i + 1].parse().ok();
                    i += 1;
                }
                _ => {}
            }
            i += 1;
        }

        entries.push(RouteEntry {
            destination: dest,
            gateway,
            mask: String::new(),
            interface: iface,
            metric,
            flags: None,
        });
    }

    Some(entries)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn macos_netif_is_not_expire_column() {
        let fixture = "Routing tables\n\nInternet:\nDestination Gateway Flags Netif Expire\ndefault 10.1.0.1 UGScg en0\n10.1.0.1 d4:6a:91:ba:18:5c UHLWIir en0 1200\n10.1.0.46 link#11 UHLWI en0 !\n";
        let routes = parse_macos_routes(fixture);
        assert_eq!(routes.len(), 3);
        assert!(routes.iter().all(|route| route.interface == "en0"));
    }
}
