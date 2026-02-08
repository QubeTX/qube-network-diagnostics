use crate::config::{Config, OutputFormat};
use crate::render::color;
use crate::render::progress::create_spinner;

use super::{fail_icon, flush_dns_platform, is_interactive, print_elevation_hint, prompt_yes_no, success_icon};

struct StepResult {
    #[allow(dead_code)]
    name: &'static str,
    success: bool,
    #[allow(dead_code)]
    message: String,
}

pub async fn run(config: &Config) -> i32 {
    if config.format == OutputFormat::Json {
        return run_json(config).await;
    }

    println!();

    let mut steps: Vec<StepResult> = Vec::new();

    // Step 1: Flush DNS
    {
        let spinner = create_spinner("Flushing DNS cache...");
        let result = flush_dns_platform().await;
        spinner.finish_and_clear();

        match result {
            Ok(msg) => {
                print_step_ok("DNS cache flushed", config);
                steps.push(StepResult { name: "dns_flush", success: true, message: msg });
            }
            Err(msg) => {
                print_step_fail("Failed to flush DNS cache", &msg, config);
                steps.push(StepResult { name: "dns_flush", success: false, message: msg });
            }
        }
    }

    // Step 2: Flush ARP cache
    {
        let spinner = create_spinner("Flushing ARP cache...");
        let result = flush_arp().await;
        spinner.finish_and_clear();

        match result {
            Ok(msg) => {
                print_step_ok("ARP cache flushed", config);
                steps.push(StepResult { name: "arp_flush", success: true, message: msg });
            }
            Err(msg) => {
                print_step_fail("Failed to flush ARP cache", &msg, config);
                steps.push(StepResult { name: "arp_flush", success: false, message: msg });
            }
        }
    }

    // Step 3: Renew DHCP lease
    {
        let spinner = create_spinner("Renewing DHCP lease...");
        let result = renew_dhcp().await;
        spinner.finish_and_clear();

        match result {
            Ok(msg) => {
                print_step_ok("DHCP lease renewed", config);
                steps.push(StepResult { name: "dhcp_renew", success: true, message: msg });
            }
            Err(msg) => {
                print_step_fail("Failed to renew DHCP lease", &msg, config);
                steps.push(StepResult { name: "dhcp_renew", success: false, message: msg });
            }
        }
    }

    // Step 4: Adapter restart (Windows/macOS only, interactive only)
    #[cfg(not(target_os = "linux"))]
    {
        let elevated = crate::platform::is_elevated();
        if elevated && is_interactive(config) {
            let adapters = list_active_adapters().await;
            if !adapters.is_empty() {
                println!();
                println!(
                    "  Active adapters: {}",
                    color::cyan(&adapters.join(", "), config),
                );
                let prompt = format!(
                    "  Would you like to restart all active network adapters? This will briefly disconnect your network. (y/N): "
                );
                if prompt_yes_no(&prompt) {
                    for adapter in &adapters {
                        let spinner = create_spinner(&format!("Restarting {}...", adapter));
                        let result = restart_adapter(adapter).await;
                        spinner.finish_and_clear();

                        match result {
                            Ok(_) => print_step_ok(&format!("Restarted {}", adapter), config),
                            Err(msg) => print_step_fail(&format!("Failed to restart {}", adapter), &msg, config),
                        }
                    }
                    steps.push(StepResult {
                        name: "adapter_restart",
                        success: true,
                        message: format!("Restarted {} adapters", adapters.len()),
                    });
                }
            }
        } else if !elevated {
            println!();
            print_elevation_hint(config);
        }
    }

    // Summary
    println!();
    let failed = steps.iter().filter(|s| !s.success).count();
    if failed == 0 {
        println!(
            "  {} {}",
            color::green(success_icon(config), config),
            color::green("Network reset complete", config),
        );
        0
    } else {
        println!(
            "  {} {}",
            color::yellow(
                if config.use_unicode {
                    crate::config::status_chars::WARN
                } else {
                    crate::config::status_chars::WARN_ASCII
                },
                config,
            ),
            color::yellow(
                &format!("Network reset complete with {} failed step(s)", failed),
                config,
            ),
        );
        2
    }
}

async fn run_json(config: &Config) -> i32 {
    let dns = match flush_dns_platform().await {
        Ok(msg) => serde_json::json!({"success": true, "message": msg}),
        Err(msg) => serde_json::json!({"success": false, "message": msg}),
    };

    let arp = match flush_arp().await {
        Ok(msg) => serde_json::json!({"success": true, "message": msg}),
        Err(msg) => serde_json::json!({"success": false, "message": msg}),
    };

    let dhcp = match renew_dhcp().await {
        Ok(msg) => serde_json::json!({"success": true, "message": msg}),
        Err(msg) => serde_json::json!({"success": false, "message": msg}),
    };

    // In JSON mode, skip adapter restart prompt entirely but attempt if elevated
    let adapter_restart = if crate::platform::is_elevated() {
        let adapters = list_active_adapters().await;
        if !adapters.is_empty() {
            let mut results = Vec::new();
            for adapter in &adapters {
                match restart_adapter(adapter).await {
                    Ok(_) => results.push(serde_json::json!({"adapter": adapter, "success": true})),
                    Err(msg) => results.push(serde_json::json!({"adapter": adapter, "success": false, "message": msg})),
                }
            }
            serde_json::json!(results)
        } else {
            serde_json::json!([])
        }
    } else {
        serde_json::json!(null)
    };

    let _config = config; // suppress unused warning

    let all_success = dns["success"].as_bool().unwrap_or(false)
        && arp["success"].as_bool().unwrap_or(false)
        && dhcp["success"].as_bool().unwrap_or(false);

    let output = serde_json::json!({
        "action": "fix",
        "dns_flush": dns,
        "arp_flush": arp,
        "dhcp_renew": dhcp,
        "adapter_restart": adapter_restart,
    });
    println!("{}", serde_json::to_string_pretty(&output).unwrap_or_else(|_| "{}".to_string()));

    if all_success { 0 } else { 2 }
}

fn print_step_ok(label: &str, config: &Config) {
    println!(
        "  {} {}",
        color::green(success_icon(config), config),
        color::green(label, config),
    );
}

fn print_step_fail(label: &str, detail: &str, config: &Config) {
    println!(
        "  {} {}",
        color::red(fail_icon(config), config),
        color::red(label, config),
    );
    if !detail.is_empty() {
        println!("    {}", color::dim(detail, config));
    }
}

// ── ARP flush ───────────────────────────────────────────────────────────────

async fn flush_arp() -> Result<String, String> {
    #[cfg(windows)]
    {
        match tokio::process::Command::new("arp")
            .args(["-d", "*"])
            .output()
            .await
        {
            Ok(output) if output.status.success() => {
                Ok("ARP cache flushed".to_string())
            }
            Ok(output) => Err(String::from_utf8_lossy(&output.stderr).trim().to_string()),
            Err(e) => Err(format!("Failed to run arp: {}", e)),
        }
    }

    #[cfg(target_os = "macos")]
    {
        match tokio::process::Command::new("arp")
            .args(["-d", "-a"])
            .output()
            .await
        {
            Ok(output) if output.status.success() => {
                Ok("ARP cache flushed".to_string())
            }
            Ok(output) => Err(String::from_utf8_lossy(&output.stderr).trim().to_string()),
            Err(e) => Err(format!("Failed to run arp: {}", e)),
        }
    }

    #[cfg(target_os = "linux")]
    {
        match tokio::process::Command::new("ip")
            .args(["neigh", "flush", "all"])
            .output()
            .await
        {
            Ok(output) if output.status.success() => {
                Ok("ARP cache flushed".to_string())
            }
            Ok(output) => Err(String::from_utf8_lossy(&output.stderr).trim().to_string()),
            Err(e) => Err(format!("Failed to run ip neigh flush: {}", e)),
        }
    }
}

// ── DHCP renew ──────────────────────────────────────────────────────────────

async fn renew_dhcp() -> Result<String, String> {
    #[cfg(windows)]
    {
        // Release first, then renew
        let release = tokio::process::Command::new("ipconfig")
            .arg("/release")
            .output()
            .await;

        if let Ok(output) = &release {
            if !output.status.success() {
                let msg = String::from_utf8_lossy(&output.stderr).trim().to_string();
                if !msg.is_empty() {
                    return Err(format!("DHCP release failed: {}", msg));
                }
            }
        }

        match tokio::process::Command::new("ipconfig")
            .arg("/renew")
            .output()
            .await
        {
            Ok(output) if output.status.success() => {
                Ok("DHCP lease renewed".to_string())
            }
            Ok(output) => Err(format!(
                "DHCP renew failed: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            )),
            Err(e) => Err(format!("Failed to run ipconfig: {}", e)),
        }
    }

    #[cfg(target_os = "macos")]
    {
        // Detect active network device
        let device = detect_active_device_macos().await;
        match &device {
            Some(dev) => {
                match tokio::process::Command::new("ipconfig")
                    .args(["set", dev, "DHCP"])
                    .output()
                    .await
                {
                    Ok(output) if output.status.success() => {
                        Ok(format!("DHCP renewed on {}", dev))
                    }
                    Ok(output) => Err(format!(
                        "DHCP renew failed on {}: {}",
                        dev,
                        String::from_utf8_lossy(&output.stderr).trim()
                    )),
                    Err(e) => Err(format!("Failed to run ipconfig: {}", e)),
                }
            }
            None => Err("Could not detect active network device".to_string()),
        }
    }

    #[cfg(target_os = "linux")]
    {
        // Try dhclient first
        let dhclient_release = tokio::process::Command::new("dhclient")
            .arg("-r")
            .output()
            .await;

        if let Ok(output) = dhclient_release {
            if output.status.success() {
                match tokio::process::Command::new("dhclient")
                    .output()
                    .await
                {
                    Ok(output) if output.status.success() => {
                        return Ok("DHCP lease renewed via dhclient".to_string());
                    }
                    _ => {}
                }
            }
        }

        // Fallback to nmcli
        match tokio::process::Command::new("nmcli")
            .args(["networking", "off"])
            .output()
            .await
        {
            Ok(off_output) if off_output.status.success() => {
                match tokio::process::Command::new("nmcli")
                    .args(["networking", "on"])
                    .output()
                    .await
                {
                    Ok(on_output) if on_output.status.success() => {
                        Ok("DHCP renewed via nmcli".to_string())
                    }
                    _ => Err("nmcli networking on failed".to_string()),
                }
            }
            Ok(_) => Err("Neither dhclient nor nmcli available".to_string()),
            Err(_) => Err("Neither dhclient nor nmcli available".to_string()),
        }
    }
}

// ── Active device detection (macOS) ─────────────────────────────────────────

#[cfg(target_os = "macos")]
async fn detect_active_device_macos() -> Option<String> {
    // Use route to find the default interface
    if let Ok(output) = tokio::process::Command::new("route")
        .args(["-n", "get", "default"])
        .output()
        .await
    {
        let text = String::from_utf8_lossy(&output.stdout);
        for line in text.lines() {
            let line = line.trim();
            if let Some(iface) = line.strip_prefix("interface:") {
                return Some(iface.trim().to_string());
            }
        }
    }
    None
}

// ── Adapter listing and restart ─────────────────────────────────────────────

#[cfg(windows)]
async fn list_active_adapters() -> Vec<String> {
    use std::collections::HashMap;
    use wmi::{COMLibrary, WMIConnection};

    tokio::task::spawn_blocking(|| {
        let com = match COMLibrary::new() {
            Ok(c) => c,
            Err(_) => return Vec::new(),
        };
        let wmi = match WMIConnection::new(com) {
            Ok(w) => w,
            Err(_) => return Vec::new(),
        };

        let query = "SELECT NetConnectionID, NetConnectionStatus FROM Win32_NetworkAdapter WHERE NetConnectionID IS NOT NULL AND NetConnectionStatus = 2";
        let results: Vec<HashMap<String, wmi::Variant>> = match wmi.raw_query(query) {
            Ok(r) => r,
            Err(_) => return Vec::new(),
        };

        results
            .into_iter()
            .filter_map(|row| match row.get("NetConnectionID") {
                Some(wmi::Variant::String(s)) => Some(s.clone()),
                _ => None,
            })
            .collect()
    })
    .await
    .unwrap_or_default()
}

#[cfg(target_os = "macos")]
async fn list_active_adapters() -> Vec<String> {
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
                // Check if this interface is active
                if let Ok(status_output) = tokio::process::Command::new("ifconfig")
                    .arg(&current_device)
                    .output()
                    .await
                {
                    let status_text = String::from_utf8_lossy(&status_output.stdout);
                    if status_text.contains("status: active") || status_text.contains("inet ") {
                        adapters.push(format!("{}:{}", current_name, current_device));
                    }
                }
            }
        }
    }
    adapters
}

#[cfg(target_os = "linux")]
async fn list_active_adapters() -> Vec<String> {
    // Linux: we don't offer adapter restart, return empty
    Vec::new()
}

#[cfg(windows)]
async fn restart_adapter(name: &str) -> Result<String, String> {
    // Disable
    let disable = tokio::process::Command::new("netsh")
        .args(["interface", "set", "interface", name, "disabled"])
        .output()
        .await;

    match disable {
        Ok(output) if !output.status.success() => {
            return Err(format!(
                "Failed to disable {}: {}",
                name,
                String::from_utf8_lossy(&output.stderr).trim()
            ));
        }
        Err(e) => return Err(format!("Failed to run netsh: {}", e)),
        _ => {}
    }

    // Wait for the adapter to fully disable
    tokio::time::sleep(std::time::Duration::from_secs(3)).await;

    // Re-enable
    match tokio::process::Command::new("netsh")
        .args(["interface", "set", "interface", name, "enabled"])
        .output()
        .await
    {
        Ok(output) if output.status.success() => Ok(format!("{} restarted", name)),
        Ok(output) => Err(format!(
            "Failed to re-enable {}: {}",
            name,
            String::from_utf8_lossy(&output.stderr).trim()
        )),
        Err(e) => Err(format!("Failed to run netsh: {}", e)),
    }
}

#[cfg(target_os = "macos")]
async fn restart_adapter(name_and_device: &str) -> Result<String, String> {
    // name_and_device is "HardwarePort:device" e.g. "Wi-Fi:en0"
    let parts: Vec<&str> = name_and_device.splitn(2, ':').collect();
    if parts.len() != 2 {
        return Err(format!("Invalid adapter format: {}", name_and_device));
    }
    let name = parts[0];
    let device = parts[1];

    let is_wifi = name.to_lowercase().contains("wi-fi") || name.to_lowercase().contains("airport");

    if is_wifi {
        // Soft toggle via networksetup
        let off = tokio::process::Command::new("networksetup")
            .args(["-setairportpower", device, "off"])
            .output()
            .await;

        if let Ok(output) = &off {
            if !output.status.success() {
                return Err(format!(
                    "Failed to disable Wi-Fi: {}",
                    String::from_utf8_lossy(&output.stderr).trim()
                ));
            }
        }

        tokio::time::sleep(std::time::Duration::from_secs(3)).await;

        match tokio::process::Command::new("networksetup")
            .args(["-setairportpower", device, "on"])
            .output()
            .await
        {
            Ok(output) if output.status.success() => Ok(format!("{} restarted", name)),
            Ok(output) => Err(format!(
                "Failed to re-enable Wi-Fi: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            )),
            Err(e) => Err(format!("Failed to run networksetup: {}", e)),
        }
    } else {
        // Hard toggle via ifconfig
        let down = tokio::process::Command::new("ifconfig")
            .args([device, "down"])
            .output()
            .await;

        if let Ok(output) = &down {
            if !output.status.success() {
                return Err(format!(
                    "Failed to disable {}: {}",
                    name,
                    String::from_utf8_lossy(&output.stderr).trim()
                ));
            }
        }

        tokio::time::sleep(std::time::Duration::from_secs(3)).await;

        match tokio::process::Command::new("ifconfig")
            .args([device, "up"])
            .output()
            .await
        {
            Ok(output) if output.status.success() => Ok(format!("{} restarted", name)),
            Ok(output) => Err(format!(
                "Failed to re-enable {}: {}",
                name,
                String::from_utf8_lossy(&output.stderr).trim()
            )),
            Err(e) => Err(format!("Failed to run ifconfig: {}", e)),
        }
    }
}

#[cfg(target_os = "linux")]
async fn restart_adapter(_name: &str) -> Result<String, String> {
    // Linux: adapter restart not offered
    Err("Adapter restart not supported on Linux".to_string())
}
