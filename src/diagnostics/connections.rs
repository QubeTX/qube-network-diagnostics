use serde::Serialize;

use super::shared_cache::SharedCache;

#[derive(Debug, Clone, Serialize)]
pub struct ConnectionEntry {
    pub protocol: String,
    pub local_addr: String,
    pub remote_addr: String,
    pub state: String,
    pub pid: Option<u32>,
    pub process_name: Option<String>,
}

pub async fn collect_with_cache(cache: &SharedCache) -> Option<Vec<ConnectionEntry>> {
    #[cfg(windows)]
    {
        if let Some(ref nc) = cache.netstat {
            return Some(parse_windows_connections(&nc.lines, &nc.process_map));
        }
    }

    #[cfg(target_os = "macos")]
    {
        if let Some(ref nc) = cache.netstat {
            return Some(parse_macos_connections(&nc.lines));
        }
    }

    // Linux uses `ss`, not netstat -ano, so always falls through
    let _ = cache;
    collect().await
}

pub async fn collect() -> Option<Vec<ConnectionEntry>> {
    #[cfg(windows)]
    {
        collect_windows().await
    }

    #[cfg(target_os = "macos")]
    {
        collect_macos().await
    }

    #[cfg(target_os = "linux")]
    {
        collect_linux().await
    }
}

#[cfg(windows)]
fn parse_windows_connections(
    lines: &[String],
    process_map: &std::collections::HashMap<u32, String>,
) -> Vec<ConnectionEntry> {
    let mut entries = Vec::new();

    for line in lines {
        let line = line.trim();
        let parts: Vec<&str> = line.split_whitespace().collect();

        if parts.len() >= 4 && (parts[0] == "TCP" || parts[0] == "UDP") {
            let pid = parts.last().and_then(|s| s.parse::<u32>().ok());
            let state = if parts[0] == "TCP" && parts.len() >= 5 {
                parts[3].to_string()
            } else {
                String::new()
            };

            let process_name = pid.and_then(|p| process_map.get(&p).cloned());

            entries.push(ConnectionEntry {
                protocol: parts[0].to_string(),
                local_addr: parts[1].to_string(),
                remote_addr: parts[2].to_string(),
                state,
                pid,
                process_name,
            });
        }
    }

    entries
}

#[cfg(windows)]
async fn collect_windows() -> Option<Vec<ConnectionEntry>> {
    use sysinfo::System;

    let output = tokio::process::Command::new("netstat")
        .args(["-ano"])
        .output()
        .await
        .ok()?;

    let text = String::from_utf8_lossy(&output.stdout);
    let lines: Vec<String> = text.lines().map(|l| l.to_string()).collect();

    let mut sys = System::new();
    sys.refresh_processes(sysinfo::ProcessesToUpdate::All, true);
    let mut process_map = std::collections::HashMap::new();
    for (pid, process) in sys.processes() {
        process_map.insert(pid.as_u32(), process.name().to_string_lossy().to_string());
    }

    Some(parse_windows_connections(&lines, &process_map))
}

#[cfg(target_os = "macos")]
fn parse_macos_connections(lines: &[String]) -> Vec<ConnectionEntry> {
    let mut entries = Vec::new();

    for line in lines {
        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.len() >= 6 && parts[0].starts_with("tcp") {
            entries.push(ConnectionEntry {
                protocol: "TCP".to_string(),
                local_addr: parts[3].to_string(),
                remote_addr: parts[4].to_string(),
                state: parts[5].to_string(),
                pid: None,
                process_name: None,
            });
        }
    }

    entries
}

#[cfg(target_os = "macos")]
async fn collect_macos() -> Option<Vec<ConnectionEntry>> {
    let output = tokio::process::Command::new("netstat")
        .args(["-anp", "tcp"])
        .output()
        .await
        .ok()?;

    let text = String::from_utf8_lossy(&output.stdout);
    let lines: Vec<String> = text.lines().map(|l| l.to_string()).collect();
    Some(parse_macos_connections(&lines))
}

#[cfg(target_os = "linux")]
async fn collect_linux() -> Option<Vec<ConnectionEntry>> {
    let output = tokio::process::Command::new("ss")
        .args(["-tupn"])
        .output()
        .await
        .ok()?;

    let text = String::from_utf8_lossy(&output.stdout);
    let mut entries = Vec::new();

    for line in text.lines().skip(1) {
        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.len() >= 5 {
            let (pid, pname) = parse_ss_process(parts.get(6).unwrap_or(&""));

            entries.push(ConnectionEntry {
                protocol: parts[0].to_uppercase(),
                local_addr: parts[4].to_string(),
                remote_addr: parts.get(5).unwrap_or(&"*:*").to_string(),
                state: parts[1].to_string(),
                pid,
                process_name: pname,
            });
        }
    }

    Some(entries)
}

#[cfg(target_os = "linux")]
fn parse_ss_process(s: &str) -> (Option<u32>, Option<String>) {
    // Format: users:(("process",pid=1234,fd=5))
    if let Some(start) = s.find("pid=") {
        let after = &s[start + 4..];
        let pid_str: String = after.chars().take_while(|c| c.is_ascii_digit()).collect();
        let pid = pid_str.parse().ok();

        let pname = if let Some(name_start) = s.find("((\"") {
            let after = &s[name_start + 3..];
            let name: String = after.chars().take_while(|c| *c != '"').collect();
            Some(name)
        } else {
            None
        };

        (pid, pname)
    } else {
        (None, None)
    }
}
