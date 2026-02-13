use serde::Serialize;
use sysinfo::Networks;

use super::shared_cache::SharedCache;

#[derive(Debug, Clone, Serialize)]
pub struct TrafficCounter {
    pub interface: String,
    pub rx_bytes: u64,
    pub tx_bytes: u64,
    pub rx_formatted: String,
    pub tx_formatted: String,
    pub total_formatted: String,
}

pub async fn collect_with_cache(cache: &SharedCache) -> Option<Vec<TrafficCounter>> {
    if let Some(ref networks) = cache.sysinfo_networks {
        return collect_from_networks(networks);
    }
    collect().await
}

fn collect_from_networks(networks: &Networks) -> Option<Vec<TrafficCounter>> {
    let mut counters = Vec::new();

    for (name, data) in networks {
        let rx = data.total_received();
        let tx = data.total_transmitted();

        if rx == 0 && tx == 0 {
            continue;
        }

        counters.push(TrafficCounter {
            interface: name.clone(),
            rx_bytes: rx,
            tx_bytes: tx,
            rx_formatted: format_bytes(rx),
            tx_formatted: format_bytes(tx),
            total_formatted: format_bytes(rx + tx),
        });
    }

    if counters.is_empty() {
        None
    } else {
        Some(counters)
    }
}

pub async fn collect() -> Option<Vec<TrafficCounter>> {
    let networks = Networks::new_with_refreshed_list();
    let mut counters = Vec::new();

    for (name, data) in &networks {
        let rx = data.total_received();
        let tx = data.total_transmitted();

        if rx == 0 && tx == 0 {
            continue;
        }

        counters.push(TrafficCounter {
            interface: name.clone(),
            rx_bytes: rx,
            tx_bytes: tx,
            rx_formatted: format_bytes(rx),
            tx_formatted: format_bytes(tx),
            total_formatted: format_bytes(rx + tx),
        });
    }

    if counters.is_empty() {
        None
    } else {
        Some(counters)
    }
}

fn format_bytes(bytes: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = 1024 * KB;
    const GB: u64 = 1024 * MB;
    const TB: u64 = 1024 * GB;

    if bytes >= TB {
        format!("{:.2} TB", bytes as f64 / TB as f64)
    } else if bytes >= GB {
        format!("{:.2} GB", bytes as f64 / GB as f64)
    } else if bytes >= MB {
        format!("{:.1} MB", bytes as f64 / MB as f64)
    } else if bytes >= KB {
        format!("{:.1} KB", bytes as f64 / KB as f64)
    } else {
        format!("{} B", bytes)
    }
}
