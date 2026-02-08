use serde::Serialize;
use sysinfo::Networks;

#[derive(Debug, Clone, Serialize)]
pub struct AdapterHwStat {
    pub name: String,
    pub rx_bytes: u64,
    pub tx_bytes: u64,
    pub rx_packets: u64,
    pub tx_packets: u64,
    pub rx_errors: u64,
    pub tx_errors: u64,
    pub link_speed: Option<String>,
    pub duplex: Option<String>,
}

pub async fn collect() -> Option<Vec<AdapterHwStat>> {
    let networks = Networks::new_with_refreshed_list();
    let mut stats = Vec::new();

    for (name, data) in &networks {
        let (link_speed, duplex) = get_link_info(name).await;

        stats.push(AdapterHwStat {
            name: name.clone(),
            rx_bytes: data.total_received(),
            tx_bytes: data.total_transmitted(),
            rx_packets: data.total_packets_received(),
            tx_packets: data.total_packets_transmitted(),
            rx_errors: data.total_errors_on_received(),
            tx_errors: data.total_errors_on_transmitted(),
            link_speed,
            duplex,
        });
    }

    if stats.is_empty() {
        None
    } else {
        Some(stats)
    }
}

async fn get_link_info(iface: &str) -> (Option<String>, Option<String>) {
    #[cfg(windows)]
    {
        // Use wmic or powershell to get link speed
        let escaped_iface = iface.replace('\'', "''");
        if let Ok(output) = tokio::process::Command::new("powershell")
            .args(["-Command", &format!(
                "Get-NetAdapter -Name '{}' | Select-Object LinkSpeed,FullDuplex | Format-List",
                escaped_iface
            )])
            .output()
            .await
        {
            let text = String::from_utf8_lossy(&output.stdout);
            let mut speed = None;
            let mut duplex = None;

            for line in text.lines() {
                let line = line.trim();
                if let Some(val) = line.strip_prefix("LinkSpeed") {
                    speed = Some(val.trim().trim_start_matches(':').trim().to_string());
                }
                if let Some(val) = line.strip_prefix("FullDuplex") {
                    let val = val.trim().trim_start_matches(':').trim();
                    duplex = Some(if val == "True" { "Full" } else { "Half" }.to_string());
                }
            }
            return (speed, duplex);
        }
        (None, None)
    }

    #[cfg(target_os = "linux")]
    {
        let speed = tokio::fs::read_to_string(format!("/sys/class/net/{}/speed", iface))
            .await
            .ok()
            .and_then(|s| {
                let mbps: u64 = s.trim().parse().ok()?;
                Some(format!("{} Mbps", mbps))
            });

        let duplex = tokio::fs::read_to_string(format!("/sys/class/net/{}/duplex", iface))
            .await
            .ok()
            .map(|s| s.trim().to_string());

        (speed, duplex)
    }

    #[cfg(target_os = "macos")]
    {
        // macOS doesn't easily expose this info
        (None, None)
    }
}
