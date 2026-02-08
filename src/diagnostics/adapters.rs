use serde::Serialize;

use super::DiagnosticResult;

#[derive(Debug, Clone, Serialize)]
pub struct AdapterInfo {
    pub name: String,
    pub adapter_type: String,
    pub status: String,
    pub has_ip: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub driver_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub driver_version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub driver_date: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub problem_code: Option<u32>,
}

pub async fn check() -> (DiagnosticResult, Vec<AdapterInfo>) {
    let adapters = collect_adapters().await;

    let error_count = adapters.iter().filter(|a| a.status == "Error").count();
    let disabled_count = adapters.iter().filter(|a| a.status == "Disabled").count();
    let active_count = adapters.iter().filter(|a| a.status == "Active").count();

    let result = if error_count > 0 {
        let names: Vec<&str> = adapters
            .iter()
            .filter(|a| a.status == "Error")
            .map(|a| a.name.as_str())
            .collect();
        DiagnosticResult::fail(
            "Adapters",
            format!(
                "{} adapter{} with errors: {}",
                error_count,
                if error_count > 1 { "s" } else { "" },
                names.join(", ")
            ),
        )
    } else if disabled_count > 0 && active_count > 0 {
        DiagnosticResult::warn(
            "Adapters",
            format!(
                "{} disabled, {} active",
                disabled_count, active_count
            ),
        )
    } else if active_count == 0 {
        DiagnosticResult::fail("Adapters", "No active network adapters found")
    } else {
        DiagnosticResult::ok("Adapters", "All network adapters healthy")
    };

    (result, adapters)
}

async fn collect_adapters() -> Vec<AdapterInfo> {
    #[cfg(windows)]
    {
        collect_adapters_windows().await
    }

    #[cfg(target_os = "macos")]
    {
        collect_adapters_macos().await
    }

    #[cfg(target_os = "linux")]
    {
        collect_adapters_linux().await
    }
}

#[cfg(windows)]
async fn collect_adapters_windows() -> Vec<AdapterInfo> {
    use std::collections::HashMap;
    use wmi::{COMLibrary, WMIConnection};

    let adapters = tokio::task::spawn_blocking(|| {
        let com = match COMLibrary::new() {
            Ok(c) => c,
            Err(_) => return Vec::new(),
        };
        let wmi = match WMIConnection::new(com) {
            Ok(w) => w,
            Err(_) => return Vec::new(),
        };

        let query = "SELECT Name, NetConnectionStatus, AdapterType, MACAddress FROM Win32_NetworkAdapter WHERE PhysicalAdapter = TRUE";
        let results: Vec<HashMap<String, wmi::Variant>> = match wmi.raw_query(query) {
            Ok(r) => r,
            Err(_) => return Vec::new(),
        };

        let mut adapters = Vec::new();
        for row in results {
            let name = match row.get("Name") {
                Some(wmi::Variant::String(s)) => s.clone(),
                _ => continue,
            };

            let status_code = match row.get("NetConnectionStatus") {
                Some(wmi::Variant::UI2(n)) => *n,
                Some(wmi::Variant::I4(n)) => *n as u16,
                _ => 0,
            };

            let adapter_type = match row.get("AdapterType") {
                Some(wmi::Variant::String(s)) => s.clone(),
                _ => "Unknown".to_string(),
            };

            let status = match status_code {
                2 => "Active".to_string(),
                1 => "Connecting".to_string(),
                0 => "Disconnected".to_string(),
                3 => "Disconnecting".to_string(),
                4..=6 => "Error".to_string(),
                7 => "Disabled".to_string(),
                _ => "Unknown".to_string(),
            };

            adapters.push(AdapterInfo {
                name,
                adapter_type,
                status,
                has_ip: status_code == 2,
                driver_name: None,
                driver_version: None,
                driver_date: None,
                problem_code: None,
            });
        }

        // Try to get driver info
        let driver_query = "SELECT Name, DriverVersion, DriverDate FROM Win32_PnPSignedDriver WHERE DeviceClass = 'NET'";
        if let Ok(drivers) = wmi.raw_query::<HashMap<String, wmi::Variant>>(driver_query) {
            for driver in drivers {
                let drv_name = match driver.get("Name") {
                    Some(wmi::Variant::String(s)) => s.clone(),
                    _ => continue,
                };
                let version = match driver.get("DriverVersion") {
                    Some(wmi::Variant::String(s)) => Some(s.clone()),
                    _ => None,
                };
                let date = match driver.get("DriverDate") {
                    Some(wmi::Variant::String(s)) => Some(s.chars().take(10).collect()),
                    _ => None,
                };

                for adapter in &mut adapters {
                    if adapter.name.contains(&drv_name) || drv_name.contains(&adapter.name) {
                        adapter.driver_name = Some(drv_name.clone());
                        adapter.driver_version = version.clone();
                        adapter.driver_date = date.clone();
                    }
                }
            }
        }

        adapters
    })
    .await
    .unwrap_or_default();

    adapters
}

#[cfg(target_os = "macos")]
async fn collect_adapters_macos() -> Vec<AdapterInfo> {
    let mut adapters = Vec::new();

    if let Ok(output) = tokio::process::Command::new("networksetup")
        .args(["-listallhardwareports"])
        .output()
        .await
    {
        let text = String::from_utf8_lossy(&output.stdout);
        let mut current_name = String::new();
        let mut current_device = String::new();

        for line in text.lines() {
            if let Some(name) = line.strip_prefix("Hardware Port: ") {
                current_name = name.trim().to_string();
            } else if let Some(dev) = line.strip_prefix("Device: ") {
                current_device = dev.trim().to_string();

                let status = if check_interface_up(&current_device).await {
                    "Active"
                } else {
                    "Disconnected"
                };

                adapters.push(AdapterInfo {
                    name: current_name.clone(),
                    adapter_type: detect_macos_type(&current_name),
                    status: status.to_string(),
                    has_ip: status == "Active",
                    driver_name: Some(current_device.clone()),
                    driver_version: None,
                    driver_date: None,
                    problem_code: None,
                });
            }
        }
    }

    adapters
}

#[cfg(target_os = "macos")]
fn detect_macos_type(name: &str) -> String {
    let lower = name.to_lowercase();
    if lower.contains("wi-fi") || lower.contains("airport") {
        "Wi-Fi".to_string()
    } else if lower.contains("ethernet") || lower.contains("thunderbolt") {
        "Ethernet".to_string()
    } else if lower.contains("bluetooth") {
        "Bluetooth".to_string()
    } else {
        "Other".to_string()
    }
}

#[cfg(target_os = "macos")]
async fn check_interface_up(device: &str) -> bool {
    if let Ok(output) = tokio::process::Command::new("ifconfig")
        .arg(device)
        .output()
        .await
    {
        let text = String::from_utf8_lossy(&output.stdout);
        text.contains("status: active") || text.contains("inet ")
    } else {
        false
    }
}

#[cfg(target_os = "linux")]
async fn collect_adapters_linux() -> Vec<AdapterInfo> {
    let mut adapters = Vec::new();

    if let Ok(output) = tokio::process::Command::new("ip")
        .args(["-o", "link", "show"])
        .output()
        .await
    {
        let text = String::from_utf8_lossy(&output.stdout);
        for line in text.lines() {
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() < 3 {
                continue;
            }

            let name = parts[1].trim_end_matches(':');
            if name == "lo" {
                continue;
            }

            let is_up = line.contains("state UP") || line.contains(",UP");
            let iface_type = if name.starts_with("wl") {
                "Wi-Fi"
            } else if name.starts_with("en") || name.starts_with("eth") {
                "Ethernet"
            } else if name.starts_with("tun") || name.starts_with("wg") {
                "VPN/Tunnel"
            } else if name.starts_with("docker") || name.starts_with("veth") || name.starts_with("br-") {
                "Virtual"
            } else {
                "Other"
            };

            // Get driver info
            let driver = get_linux_driver(name).await;

            adapters.push(AdapterInfo {
                name: name.to_string(),
                adapter_type: iface_type.to_string(),
                status: if is_up { "Active" } else { "Disconnected" }.to_string(),
                has_ip: is_up,
                driver_name: driver,
                driver_version: None,
                driver_date: None,
                problem_code: None,
            });
        }
    }

    adapters
}

#[cfg(target_os = "linux")]
async fn get_linux_driver(iface: &str) -> Option<String> {
    let path = format!("/sys/class/net/{}/device/driver", iface);
    if let Ok(link) = tokio::fs::read_link(&path).await {
        link.file_name()
            .and_then(|n| n.to_str())
            .map(|s| s.to_string())
    } else {
        None
    }
}
