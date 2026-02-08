use serde::Serialize;
use std::time::Instant;

use super::DiagnosticResult;

#[derive(Debug, Clone, Serialize)]
pub struct PublicIpInfo {
    pub ip: String,
    pub lookup_time_ms: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub city: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub region: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub country: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub isp: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub org: Option<String>,
    pub behind_nat: bool,
}

pub async fn check() -> (DiagnosticResult, Option<PublicIpInfo>) {
    let start = Instant::now();

    let client = match reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()
    {
        Ok(c) => c,
        Err(_) => {
            return (
                DiagnosticResult::fail("Internet", "Failed to create HTTP client"),
                None,
            );
        }
    };

    // Try Cloudflare trace first (fast, no rate limits)
    let ip = match get_ip_cloudflare(&client).await {
        Some(ip) => ip,
        None => {
            // Fallback to ipify
            match get_ip_ipify(&client).await {
                Some(ip) => ip,
                None => {
                    return (
                        DiagnosticResult::fail("Internet", "Could not determine public IP"),
                        None,
                    );
                }
            }
        }
    };

    let lookup_time = start.elapsed().as_secs_f64() * 1000.0;

    // Get geolocation
    let geo = get_geolocation(&client, &ip).await;

    // Check NAT
    let local_ips = get_local_ips();
    let behind_nat = !local_ips.contains(&ip);

    let info = PublicIpInfo {
        ip: ip.clone(),
        lookup_time_ms: lookup_time,
        city: geo.as_ref().and_then(|g| g.city.clone()),
        region: geo.as_ref().and_then(|g| g.region.clone()),
        country: geo.as_ref().and_then(|g| g.country.clone()),
        isp: geo.as_ref().and_then(|g| g.isp.clone()),
        org: geo.as_ref().and_then(|g| g.org.clone()),
        behind_nat,
    };

    let mut summary = "Public IP obtained".to_string();
    if let Some(ref g) = geo {
        if let (Some(ref city), Some(ref country)) = (&g.city, &g.country) {
            if !city.is_empty() {
                summary = format!("{}, {}", city, country);
            }
        }
    }

    (DiagnosticResult::ok("Internet", summary), Some(info))
}

async fn get_ip_cloudflare(client: &reqwest::Client) -> Option<String> {
    let resp = client
        .get("https://1.1.1.1/cdn-cgi/trace")
        .send()
        .await
        .ok()?;
    let text = resp.text().await.ok()?;

    for line in text.lines() {
        if let Some(ip) = line.strip_prefix("ip=") {
            return Some(ip.trim().to_string());
        }
    }
    None
}

async fn get_ip_ipify(client: &reqwest::Client) -> Option<String> {
    let resp = client
        .get("https://api.ipify.org")
        .send()
        .await
        .ok()?;
    let text = resp.text().await.ok()?;
    Some(text.trim().to_string())
}

#[derive(Debug, Clone)]
struct GeoInfo {
    city: Option<String>,
    region: Option<String>,
    country: Option<String>,
    isp: Option<String>,
    org: Option<String>,
}

async fn get_geolocation(client: &reqwest::Client, ip: &str) -> Option<GeoInfo> {
    let url = format!("http://ip-api.com/json/{}?fields=city,regionName,country,isp,org", ip);

    let resp = client.get(&url).send().await.ok()?;
    let json: serde_json::Value = resp.json().await.ok()?;

    Some(GeoInfo {
        city: json.get("city").and_then(|v| v.as_str()).map(|s| s.to_string()),
        region: json.get("regionName").and_then(|v| v.as_str()).map(|s| s.to_string()),
        country: json.get("country").and_then(|v| v.as_str()).map(|s| s.to_string()),
        isp: json.get("isp").and_then(|v| v.as_str()).map(|s| s.to_string()),
        org: json.get("org").and_then(|v| v.as_str()).map(|s| s.to_string()),
    })
}

fn get_local_ips() -> Vec<String> {
    let networks = sysinfo::Networks::new_with_refreshed_list();
    let mut ips = Vec::new();
    for (_name, data) in &networks {
        for net in data.ip_networks() {
            ips.push(net.addr.to_string());
        }
    }
    ips
}
