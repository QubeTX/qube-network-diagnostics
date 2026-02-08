use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct DnsCacheEntry {
    pub name: String,
    pub record_type: String,
    pub data: String,
    pub ttl: Option<u32>,
}

pub async fn collect() -> Option<Vec<DnsCacheEntry>> {
    #[cfg(windows)]
    {
        collect_windows().await
    }

    #[cfg(target_os = "macos")]
    {
        // macOS doesn't have an easy way to dump DNS cache
        None
    }

    #[cfg(target_os = "linux")]
    {
        collect_linux().await
    }
}

#[cfg(windows)]
async fn collect_windows() -> Option<Vec<DnsCacheEntry>> {
    let output = tokio::process::Command::new("ipconfig")
        .args(["/displaydns"])
        .output()
        .await
        .ok()?;

    let text = String::from_utf8_lossy(&output.stdout);
    let mut entries = Vec::new();
    let mut current_name = String::new();
    let mut current_type = String::new();
    let mut current_ttl = None;

    for line in text.lines() {
        let line = line.trim();

        if line.contains("Record Name") {
            current_name = line.split(':').nth(1).map(|s| s.trim().to_string()).unwrap_or_default();
        } else if line.contains("Record Type") {
            let type_num: u32 = line
                .split(':')
                .nth(1)
                .and_then(|s| s.trim().parse().ok())
                .unwrap_or(0);
            current_type = match type_num {
                1 => "A",
                5 => "CNAME",
                28 => "AAAA",
                12 => "PTR",
                15 => "MX",
                _ => "OTHER",
            }.to_string();
        } else if line.contains("Time To Live") {
            current_ttl = line.split(':').nth(1).and_then(|s| s.trim().parse().ok());
        } else if line.contains("A (Host) Record") || line.contains("CNAME Record") || line.contains("AAAA Record") {
            let data = line.splitn(2, ':').nth(1).unwrap_or("").trim().to_string();
            if !current_name.is_empty() {
                entries.push(DnsCacheEntry {
                    name: current_name.clone(),
                    record_type: current_type.clone(),
                    data,
                    ttl: current_ttl,
                });
            }
        }
    }

    // Limit to prevent overwhelming output
    entries.truncate(50);

    if entries.is_empty() {
        None
    } else {
        Some(entries)
    }
}

#[cfg(target_os = "linux")]
async fn collect_linux() -> Option<Vec<DnsCacheEntry>> {
    // Try resolvectl
    if let Ok(output) = tokio::process::Command::new("resolvectl")
        .args(["statistics"])
        .output()
        .await
    {
        let text = String::from_utf8_lossy(&output.stdout);
        if !text.is_empty() {
            // resolvectl doesn't dump individual entries easily
            // Just return stats-based info
            let mut entries = Vec::new();
            for line in text.lines() {
                if line.contains("Current Cache Size") {
                    entries.push(DnsCacheEntry {
                        name: "Cache Statistics".to_string(),
                        record_type: "INFO".to_string(),
                        data: line.trim().to_string(),
                        ttl: None,
                    });
                }
            }
            if !entries.is_empty() {
                return Some(entries);
            }
        }
    }

    None
}
