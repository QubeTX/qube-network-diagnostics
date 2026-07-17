use std::collections::HashMap;

/// Pre-fetched data shared across technician-mode diagnostic modules.
/// Built once before `tokio::join!` and passed by reference into each
/// module's `collect_with_cache()` to avoid duplicate subprocess calls.
pub struct SharedCache {
    pub netstat: Option<NetstatCache>,
    /// Machine-readable `lsof` TCP records used on macOS when Darwin's
    /// fixed-width `netstat` output truncates long IPv6 endpoints. Always
    /// `None` on other platforms.
    pub macos_lsof_tcp: Option<Vec<String>>,
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
        let (netstat, macos_lsof_tcp, ipconfig, networks, gateway) = tokio::join!(
            fetch_netstat(),
            fetch_macos_lsof_tcp_lines(),
            fetch_ipconfig(),
            fetch_sysinfo_networks(),
            fetch_gateway(),
        );

        Self {
            netstat,
            macos_lsof_tcp,
            ipconfig,
            sysinfo_networks: networks,
            gateway_ip: gateway,
        }
    }
}

/// Darwin 25/macOS 26 still truncates long IPv6 addresses in `netstat` even
/// with `-W`. `lsof` field output is stable, machine-readable, and retains the
/// complete bracketed endpoints. The shared diagnostic timeout kills and
/// reaps `lsof` if the kernel query stalls.
#[cfg(target_os = "macos")]
pub(super) async fn fetch_macos_lsof_tcp_lines() -> Option<Vec<String>> {
    let mut cmd = tokio::process::Command::new("/usr/sbin/lsof");
    cmd.args(["-nP", "-iTCP", "-FpcPnT"]);
    let output = super::util::run_with_timeout(cmd, super::util::QUICK).await?;
    if !output.status.success() {
        return None;
    }

    let lines: Vec<String> = String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(str::to_string)
        .collect();
    (!lines.is_empty()).then_some(lines)
}

#[cfg(not(target_os = "macos"))]
async fn fetch_macos_lsof_tcp_lines() -> Option<Vec<String>> {
    None
}

// ── Netstat ────────────────────────────────────────────────────────────────

#[cfg(windows)]
async fn fetch_netstat() -> Option<NetstatCache> {
    use sysinfo::System;

    let mut cmd = tokio::process::Command::new("netstat");
    cmd.args(["-ano"]);
    let output = super::util::run_with_timeout(cmd, super::util::QUICK).await?;

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
    let mut cmd = tokio::process::Command::new("netstat");
    // `-W` is still useful for Darwin's other wide columns. Long IPv6
    // endpoints remain fixed-width on macOS 26, so `connections` prefers the
    // bounded lsof field cache above when it is available.
    cmd.args(["-anW", "-p", "tcp"]);
    let output = super::util::run_with_timeout(cmd, super::util::QUICK).await?;

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
    let mut cmd = tokio::process::Command::new("netstat");
    cmd.args(["-an"]);
    let output = super::util::run_with_timeout(cmd, super::util::QUICK).await?;

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
    let mut cmd = tokio::process::Command::new("ipconfig");
    cmd.args(["/all"]);
    let output = super::util::run_with_timeout(cmd, super::util::QUICK).await?;

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
    // Blocking system enumeration — run it off the async runtime. `Networks`
    // is Send, so the object itself crosses back (the cache contract needs the
    // whole object). A JoinError falls back to None, identical to a fetch
    // failure, so consumers use their own subprocess fallback.
    tokio::task::spawn_blocking(|| Some(sysinfo::Networks::new_with_refreshed_list()))
        .await
        .unwrap_or(None)
}

// ── Gateway IP ─────────────────────────────────────────────────────────────

async fn fetch_gateway() -> Option<String> {
    // Blocking syscall — run it off the async runtime. A JoinError falls back
    // to None, identical to the Err path.
    tokio::task::spawn_blocking(|| {
        default_net::get_default_gateway()
            .ok()
            .map(|gw| gw.ip_addr.to_string())
    })
    .await
    .unwrap_or(None)
}
