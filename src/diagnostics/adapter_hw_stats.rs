use serde::Serialize;
use sysinfo::Networks;

use super::shared_cache::SharedCache;

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

pub async fn collect_with_cache(cache: &SharedCache) -> Option<Vec<AdapterHwStat>> {
    if let Some(ref networks) = cache.sysinfo_networks {
        return collect_from_networks(networks).await;
    }
    collect().await
}

async fn collect_from_networks(networks: &Networks) -> Option<Vec<AdapterHwStat>> {
    let mut stats = Vec::new();

    for (name, data) in networks {
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

async fn get_link_info(_iface: &str) -> (Option<String>, Option<String>) {
    #[cfg(windows)]
    {
        // Look up interface by name via ipconfig crate, then use GetIfEntry2
        let iface_name = _iface.to_string();
        tokio::task::spawn_blocking(move || {
            let adapters = match ipconfig::get_adapters() {
                Ok(a) => a,
                Err(_) => return (None, None),
            };

            // Match by friendly_name
            let adapter = adapters.iter().find(|a| a.friendly_name() == iface_name);

            let adapter = match adapter {
                Some(a) => a,
                None => return (None, None),
            };

            let if_index = adapter.ipv6_if_index();
            if if_index == 0 {
                // Fall back to link speed from ipconfig crate
                let tx = adapter.transmit_link_speed();
                let speed = if tx > 0 {
                    Some(format_speed_bps(tx))
                } else {
                    None
                };
                return (speed, None);
            }

            // Call GetIfEntry2 for precise speed + duplex
            use std::mem::zeroed;
            use winapi::shared::netioapi::{GetIfEntry2, MIB_IF_ROW2};

            unsafe {
                let mut row: MIB_IF_ROW2 = zeroed();
                row.InterfaceIndex = if_index;
                let ret = GetIfEntry2(&mut row);
                if ret != 0 {
                    return (None, None);
                }

                let tx_speed = row.TransmitLinkSpeed;
                let speed = if tx_speed > 0 {
                    Some(format_speed_bps(tx_speed))
                } else {
                    None
                };

                // MIB_IF_ROW2 doesn't directly expose duplex, but
                // ConnectionType can hint: 1=dedicated (usually full duplex)
                let duplex = match row.ConnectionType {
                    1 => Some("Full".to_string()), // NET_IF_CONNECTION_DEDICATED
                    _ => None,
                };

                (speed, duplex)
            }
        })
        .await
        .unwrap_or((None, None))
    }

    #[cfg(target_os = "linux")]
    {
        let speed = tokio::fs::read_to_string(format!("/sys/class/net/{}/speed", _iface))
            .await
            .ok()
            .and_then(|s| {
                let mbps: u64 = s.trim().parse().ok()?;
                Some(format!("{} Mbps", mbps))
            });

        let duplex = tokio::fs::read_to_string(format!("/sys/class/net/{}/duplex", _iface))
            .await
            .ok()
            .map(|s| s.trim().to_string());

        (speed, duplex)
    }

    #[cfg(target_os = "macos")]
    {
        (None, None)
    }
}

#[cfg(windows)]
fn format_speed_bps(bps: u64) -> String {
    let mbps = bps / 1_000_000;
    if mbps >= 1000 {
        format!("{:.1} Gbps", mbps as f64 / 1000.0)
    } else {
        format!("{} Mbps", mbps)
    }
}
