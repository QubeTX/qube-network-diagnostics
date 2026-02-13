use serde::Serialize;

use super::shared_cache::SharedCache;

#[derive(Debug, Clone, Serialize)]
pub struct ListeningPort {
    pub protocol: String,
    pub address: String,
    pub port: u16,
    pub pid: Option<u32>,
    pub process_name: Option<String>,
}

pub async fn collect_with_cache(cache: &SharedCache) -> Option<Vec<ListeningPort>> {
    #[cfg(windows)]
    {
        if let Some(ref nc) = cache.netstat {
            return Some(parse_windows_listeners(&nc.lines, &nc.process_map));
        }
    }

    #[cfg(target_os = "macos")]
    {
        if let Some(ref nc) = cache.netstat {
            return Some(parse_macos_listeners(&nc.lines));
        }
    }

    let _ = cache;
    collect().await
}

pub async fn collect() -> Option<Vec<ListeningPort>> {
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
fn parse_windows_listeners(
    lines: &[String],
    process_map: &std::collections::HashMap<u32, String>,
) -> Vec<ListeningPort> {
    let mut ports = Vec::new();

    for line in lines {
        let line = line.trim();
        if !line.contains("LISTENING") && !(line.starts_with("UDP") && line.contains("*:*")) {
            // Also match lines that start with whitespace before UDP
            if !(line.trim_start().starts_with("UDP") && line.contains("*:*")) {
                continue;
            }
        }
        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.len() < 4 {
            continue;
        }

        let protocol = parts[0].to_string();
        let local = parts[1];
        let pid = parts.last().and_then(|s| s.parse::<u32>().ok());

        if let Some((addr, port)) = parse_addr_port(local) {
            let pname = pid.and_then(|p| process_map.get(&p).cloned());

            ports.push(ListeningPort {
                protocol,
                address: addr,
                port,
                pid,
                process_name: pname,
            });
        }
    }

    ports
}

#[cfg(windows)]
async fn collect_windows() -> Option<Vec<ListeningPort>> {
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

    Some(parse_windows_listeners(&lines, &process_map))
}

#[cfg(target_os = "macos")]
fn parse_macos_listeners(lines: &[String]) -> Vec<ListeningPort> {
    let mut ports = Vec::new();

    for line in lines {
        if !line.contains("LISTEN") {
            continue;
        }
        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.len() >= 4 {
            if let Some((addr, port)) = parse_addr_port(parts[3]) {
                ports.push(ListeningPort {
                    protocol: "TCP".to_string(),
                    address: addr,
                    port,
                    pid: None,
                    process_name: None,
                });
            }
        }
    }

    ports
}

#[cfg(target_os = "macos")]
async fn collect_macos() -> Option<Vec<ListeningPort>> {
    let output = tokio::process::Command::new("netstat")
        .args(["-anp", "tcp"])
        .output()
        .await
        .ok()?;

    let text = String::from_utf8_lossy(&output.stdout);
    let lines: Vec<String> = text.lines().map(|l| l.to_string()).collect();
    Some(parse_macos_listeners(&lines))
}

#[cfg(target_os = "linux")]
async fn collect_linux() -> Option<Vec<ListeningPort>> {
    let output = tokio::process::Command::new("ss")
        .args(["-tulnp"])
        .output()
        .await
        .ok()?;

    let text = String::from_utf8_lossy(&output.stdout);
    let mut ports = Vec::new();

    for line in text.lines().skip(1) {
        if !line.contains("LISTEN") && !line.contains("UNCONN") {
            continue;
        }
        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.len() >= 5 {
            if let Some((addr, port)) = parse_addr_port(parts[4]) {
                let (pid, pname) = if parts.len() > 6 {
                    parse_ss_process(parts[6])
                } else {
                    (None, None)
                };

                ports.push(ListeningPort {
                    protocol: parts[0].to_uppercase(),
                    address: addr,
                    port,
                    pid,
                    process_name: pname,
                });
            }
        }
    }

    Some(ports)
}

fn parse_addr_port(s: &str) -> Option<(String, u16)> {
    // Handle [::]:port and addr:port
    if let Some(bracket_end) = s.rfind("]:") {
        let addr = s[..bracket_end + 1].to_string();
        let port_str = &s[bracket_end + 2..];
        let port = port_str.parse().ok()?;
        Some((addr, port))
    } else if let Some(colon) = s.rfind(':') {
        let addr = s[..colon].to_string();
        let port = s[colon + 1..].parse().ok()?;
        Some((addr, port))
    } else {
        None
    }
}

#[cfg(target_os = "linux")]
fn parse_ss_process(s: &str) -> (Option<u32>, Option<String>) {
    if let Some(start) = s.find("pid=") {
        let after = &s[start + 4..];
        let pid_str: String = after.chars().take_while(|c| c.is_ascii_digit()).collect();
        let pid = pid_str.parse().ok();
        let pname = s.find("((\"").and_then(|ns| {
            let after = &s[ns + 3..];
            Some(after.chars().take_while(|c| *c != '"').collect())
        });
        (pid, pname)
    } else {
        (None, None)
    }
}
