use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct FirewallInfo {
    pub enabled: bool,
    pub profiles: Vec<FirewallProfile>,
    pub summary: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct FirewallProfile {
    pub name: String,
    pub enabled: bool,
    pub default_inbound: Option<String>,
    pub default_outbound: Option<String>,
}

pub async fn collect() -> Option<FirewallInfo> {
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
async fn collect_windows() -> Option<FirewallInfo> {
    let output = tokio::process::Command::new("netsh")
        .args(["advfirewall", "show", "allprofiles", "state"])
        .output()
        .await
        .ok()?;

    let text = String::from_utf8_lossy(&output.stdout);
    let mut profiles = Vec::new();
    let mut current_name = String::new();

    for line in text.lines() {
        let line = line.trim();
        if (line.contains("Profile Settings") || line.contains("Profile")) && line.ends_with(':') {
            current_name = line
                .replace("Profile Settings:", "")
                .replace("Profile:", "")
                .trim()
                .to_string();
            if current_name.is_empty() {
                current_name = "Unknown".to_string();
            }
        }
        if line.contains("State") {
            let enabled = line.contains("ON");
            profiles.push(FirewallProfile {
                name: current_name.clone(),
                enabled,
                default_inbound: None,
                default_outbound: None,
            });
        }
    }

    // Get policy info
    if let Ok(output) = tokio::process::Command::new("netsh")
        .args(["advfirewall", "show", "allprofiles"])
        .output()
        .await
    {
        let text = String::from_utf8_lossy(&output.stdout);
        let mut idx = 0;

        for line in text.lines() {
            let line = line.trim();
            if line.contains("Firewall Policy") {
                if let Some(val) = line.split_whitespace().last() {
                    if idx < profiles.len() {
                        let parts: Vec<&str> = val.split(',').collect();
                        profiles[idx].default_inbound = parts.first().map(|s| s.to_string());
                        profiles[idx].default_outbound = parts.get(1).map(|s| s.to_string());
                    }
                    idx += 1;
                }
            }
        }
    }

    let any_enabled = profiles.iter().any(|p| p.enabled);
    let summary = if profiles.is_empty() {
        "Could not determine firewall status".to_string()
    } else if any_enabled {
        let enabled: Vec<&str> = profiles
            .iter()
            .filter(|p| p.enabled)
            .map(|p| p.name.as_str())
            .collect();
        format!("Active: {}", enabled.join(", "))
    } else {
        "All profiles disabled".to_string()
    };

    Some(FirewallInfo {
        enabled: any_enabled,
        profiles,
        summary,
    })
}

#[cfg(target_os = "macos")]
async fn collect_macos() -> Option<FirewallInfo> {
    let output = tokio::process::Command::new("/usr/libexec/ApplicationFirewall/socketfilterfw")
        .args(["--getglobalstate"])
        .output()
        .await
        .ok()?;

    let text = String::from_utf8_lossy(&output.stdout);
    let enabled = text.contains("enabled");

    let mut stealth = false;
    if let Ok(output) =
        tokio::process::Command::new("/usr/libexec/ApplicationFirewall/socketfilterfw")
            .args(["--getstealthmode"])
            .output()
            .await
    {
        let text = String::from_utf8_lossy(&output.stdout);
        stealth = text.contains("enabled");
    }

    let summary = if enabled {
        if stealth {
            "Application Firewall enabled (stealth mode on)".to_string()
        } else {
            "Application Firewall enabled".to_string()
        }
    } else {
        "Application Firewall disabled".to_string()
    };

    Some(FirewallInfo {
        enabled,
        profiles: vec![FirewallProfile {
            name: "Application Firewall".to_string(),
            enabled,
            default_inbound: None,
            default_outbound: None,
        }],
        summary,
    })
}

#[cfg(target_os = "linux")]
async fn collect_linux() -> Option<FirewallInfo> {
    // Try ufw first
    if let Ok(output) = tokio::process::Command::new("ufw")
        .args(["status"])
        .output()
        .await
    {
        let text = String::from_utf8_lossy(&output.stdout);
        if text.contains("Status:") {
            let enabled = text.contains("active");
            return Some(FirewallInfo {
                enabled,
                profiles: vec![FirewallProfile {
                    name: "UFW".to_string(),
                    enabled,
                    default_inbound: None,
                    default_outbound: None,
                }],
                summary: if enabled {
                    "UFW active".to_string()
                } else {
                    "UFW inactive".to_string()
                },
            });
        }
    }

    // Try iptables
    if let Ok(output) = tokio::process::Command::new("iptables")
        .args(["-L", "-n", "--line-numbers"])
        .output()
        .await
    {
        let text = String::from_utf8_lossy(&output.stdout);
        let rule_count = text
            .lines()
            .filter(|l| l.starts_with(char::is_numeric))
            .count();
        return Some(FirewallInfo {
            enabled: rule_count > 0,
            profiles: vec![FirewallProfile {
                name: "iptables".to_string(),
                enabled: rule_count > 0,
                default_inbound: None,
                default_outbound: None,
            }],
            summary: format!("iptables: {} rules", rule_count),
        });
    }

    Some(FirewallInfo {
        enabled: false,
        profiles: vec![],
        summary: "No firewall detected".to_string(),
    })
}
