use serde::Serialize;

use super::shared_cache::SharedCache;

#[derive(Debug, Clone, Serialize)]
pub struct ReverseDnsEntry {
    pub ip: String,
    pub hostname: Option<String>,
    pub label: String,
}

pub async fn collect_with_cache(cache: &SharedCache) -> Option<Vec<ReverseDnsEntry>> {
    let mut entries = Vec::new();

    let gateway_ip = cache.gateway_ip.clone();

    let mut ips_to_check: Vec<(String, String)> = Vec::new();

    if let Some(ref gw) = gateway_ip {
        ips_to_check.push((gw.clone(), "Gateway".to_string()));
    }

    ips_to_check.push(("1.1.1.1".to_string(), "Cloudflare DNS".to_string()));
    ips_to_check.push(("8.8.8.8".to_string(), "Google DNS".to_string()));

    for (ip, label) in ips_to_check {
        let hostname = reverse_lookup(&ip).await;
        entries.push(ReverseDnsEntry {
            ip,
            hostname,
            label,
        });
    }

    if entries.is_empty() {
        None
    } else {
        Some(entries)
    }
}

pub async fn collect() -> Option<Vec<ReverseDnsEntry>> {
    let mut entries = Vec::new();

    // Get gateway IP
    let gateway_ip = default_net::get_default_gateway()
        .ok()
        .map(|gw| gw.ip_addr.to_string());

    // Collect IPs to reverse-lookup
    let mut ips_to_check: Vec<(String, String)> = Vec::new();

    if let Some(ref gw) = gateway_ip {
        ips_to_check.push((gw.clone(), "Gateway".to_string()));
    }

    ips_to_check.push(("1.1.1.1".to_string(), "Cloudflare DNS".to_string()));
    ips_to_check.push(("8.8.8.8".to_string(), "Google DNS".to_string()));

    for (ip, label) in ips_to_check {
        let hostname = reverse_lookup(&ip).await;
        entries.push(ReverseDnsEntry {
            ip,
            hostname,
            label,
        });
    }

    if entries.is_empty() {
        None
    } else {
        Some(entries)
    }
}

async fn reverse_lookup(ip: &str) -> Option<String> {
    #[cfg(windows)]
    {
        let output = tokio::process::Command::new("nslookup")
            .args([ip])
            .output()
            .await
            .ok()?;

        let text = String::from_utf8_lossy(&output.stdout);
        for line in text.lines() {
            if line.trim().starts_with("Name:") {
                return line.split(':').nth(1).map(|s| s.trim().to_string());
            }
        }
        None
    }

    #[cfg(unix)]
    {
        let output = tokio::process::Command::new("dig")
            .args(["-x", ip, "+short"])
            .output()
            .await
            .ok()?;

        let text = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if text.is_empty() {
            None
        } else {
            Some(text.trim_end_matches('.').to_string())
        }
    }
}
