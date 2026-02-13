use std::collections::HashMap;

/// Pre-fetched data shared across technician-mode diagnostic modules.
/// Built once before `tokio::join!` and passed by reference into each
/// module's `collect_with_cache()` to avoid duplicate subprocess calls.
pub struct SharedCache {
    pub netstat: Option<NetstatCache>,
    pub ipconfig: Option<IpconfigCache>,
    pub sysinfo_networks: Option<sysinfo::Networks>,
    pub gateway_ip: Option<String>,
}

/// Parsed `netstat -ano` output plus PID-to-process-name map.
pub struct NetstatCache {
    pub lines: Vec<String>,
    pub process_map: HashMap<u32, String>,
}

/// `ipconfig /all` output stored as raw text.
pub struct IpconfigCache {
    pub raw: String,
}

impl SharedCache {
    /// Run all pre-fetches concurrently. Each field is `None` on failure
    /// so consumers can fall back to their own subprocess call.
    pub async fn build_for_tech_mode() -> Self {
        let (netstat, ipconfig, networks, gateway) = tokio::join!(
            fetch_netstat(),
            fetch_ipconfig(),
            fetch_sysinfo_networks(),
            fetch_gateway(),
        );

        Self {
            netstat,
            ipconfig,
            sysinfo_networks: networks,
            gateway_ip: gateway,
        }
    }
}

// ── Netstat ────────────────────────────────────────────────────────────────

#[cfg(windows)]
async fn fetch_netstat() -> Option<NetstatCache> {
    use sysinfo::System;

    let output = tokio::process::Command::new("netstat")
        .args(["-ano"])
        .output()
        .await
        .ok()?;

    let text = String::from_utf8_lossy(&output.stdout);
    let lines: Vec<String> = text.lines().map(|l| l.to_string()).collect();

    // Build PID->process-name map from a single sysinfo refresh
    let process_map = tokio::task::spawn_blocking(|| {
        let mut sys = System::new();
        sys.refresh_processes(sysinfo::ProcessesToUpdate::All, true);
        let mut map = HashMap::new();
        for (pid, process) in sys.processes() {
            map.insert(pid.as_u32(), process.name().to_string_lossy().to_string());
        }
        map
    })
    .await
    .unwrap_or_default();

    Some(NetstatCache { lines, process_map })
}

#[cfg(target_os = "macos")]
async fn fetch_netstat() -> Option<NetstatCache> {
    let output = tokio::process::Command::new("netstat")
        .args(["-anp", "tcp"])
        .output()
        .await
        .ok()?;

    let text = String::from_utf8_lossy(&output.stdout);
    let lines: Vec<String> = text.lines().map(|l| l.to_string()).collect();

    Some(NetstatCache {
        lines,
        process_map: HashMap::new(),
    })
}

#[cfg(target_os = "linux")]
async fn fetch_netstat() -> Option<NetstatCache> {
    // Linux modules use `ss` and `netstat -an`, not `netstat -ano`.
    // We still fetch `netstat -an` for connection_states.
    let output = tokio::process::Command::new("netstat")
        .args(["-an"])
        .output()
        .await
        .ok()?;

    let text = String::from_utf8_lossy(&output.stdout);
    let lines: Vec<String> = text.lines().map(|l| l.to_string()).collect();

    Some(NetstatCache {
        lines,
        process_map: HashMap::new(),
    })
}

// ── Ipconfig ───────────────────────────────────────────────────────────────

#[cfg(windows)]
async fn fetch_ipconfig() -> Option<IpconfigCache> {
    let output = tokio::process::Command::new("ipconfig")
        .args(["/all"])
        .output()
        .await
        .ok()?;

    let raw = String::from_utf8_lossy(&output.stdout).to_string();
    Some(IpconfigCache { raw })
}

#[cfg(not(windows))]
async fn fetch_ipconfig() -> Option<IpconfigCache> {
    // ipconfig /all is Windows-only; other platforms don't use it
    None
}

// ── Sysinfo Networks ──────────────────────────────────────────────────────

async fn fetch_sysinfo_networks() -> Option<sysinfo::Networks> {
    Some(sysinfo::Networks::new_with_refreshed_list())
}

// ── Gateway IP ─────────────────────────────────────────────────────────────

async fn fetch_gateway() -> Option<String> {
    default_net::get_default_gateway()
        .ok()
        .map(|gw| gw.ip_addr.to_string())
}
