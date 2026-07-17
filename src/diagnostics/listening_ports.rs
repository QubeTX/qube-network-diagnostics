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
        if !(line.contains("LISTENING") || line.starts_with("UDP") && line.contains("*:*")) {
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

    let mut cmd = tokio::process::Command::new("netstat");
    cmd.args(["-ano"]);
    let output = super::util::run_with_timeout(cmd, super::util::QUICK).await?;

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

#[cfg(any(target_os = "macos", test))]
fn parse_macos_listeners(lines: &[String]) -> Vec<ListeningPort> {
    let mut ports = Vec::new();

    for line in lines {
        if !line.contains("LISTEN") {
            continue;
        }
        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.len() >= 4 && parts[0].starts_with("tcp") {
            if let Some((addr, port)) = parse_macos_addr_port(parts[3]) {
                ports.push(ListeningPort {
                    // Keep the Darwin family marker so distinct TCP4/TCP6 (or
                    // a combined TCP46) bindings are not presented as
                    // accidental duplicate generic TCP rows.
                    protocol: parts[0].to_ascii_uppercase(),
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

/// Darwin netstat writes endpoints as `127.0.0.1.631`, `::1.631`, and
/// `*.631` rather than the `address:port` notation used by Windows/Linux.
#[cfg(any(target_os = "macos", test))]
fn parse_macos_addr_port(value: &str) -> Option<(String, u16)> {
    let (address, port) = value.rsplit_once('.')?;
    let port = port.parse().ok()?;
    Some((address.to_string(), port))
}

#[cfg(target_os = "macos")]
async fn collect_macos() -> Option<Vec<ListeningPort>> {
    let mut cmd = tokio::process::Command::new("netstat");
    cmd.args(["-anW", "-p", "tcp"]);
    let output = super::util::run_with_timeout(cmd, super::util::QUICK).await?;

    let text = String::from_utf8_lossy(&output.stdout);
    let lines: Vec<String> = text.lines().map(|l| l.to_string()).collect();
    Some(parse_macos_listeners(&lines))
}

#[cfg(target_os = "linux")]
async fn collect_linux() -> Option<Vec<ListeningPort>> {
    let mut cmd = tokio::process::Command::new("ss");
    cmd.args(["-tulnp"]);
    let output = super::util::run_with_timeout(cmd, super::util::QUICK).await?;

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

#[cfg(any(windows, target_os = "linux", test))]
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
        let pname = s.find("((\"").map(|ns| {
            let after = &s[ns + 3..];
            after.chars().take_while(|c| *c != '"').collect()
        });
        (pid, pname)
    } else {
        (None, None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_darwin_dotted_endpoints() {
        assert_eq!(
            parse_macos_addr_port("127.0.0.1.631"),
            Some(("127.0.0.1".to_string(), 631))
        );
        assert_eq!(
            parse_macos_addr_port("::1.4318"),
            Some(("::1".to_string(), 4318))
        );
        assert_eq!(parse_macos_addr_port("*.22"), Some(("*".to_string(), 22)));
        assert_eq!(parse_macos_addr_port("*.*"), None);
        assert_eq!(parse_addr_port("[::]:443"), Some(("[::]".to_string(), 443)));
    }

    #[test]
    fn parses_macos_listener_rows() {
        let lines = vec![
            "tcp4 0 0 127.0.0.1.631 *.* LISTEN".to_string(),
            "tcp6 0 0 ::1.4318 *.* LISTEN".to_string(),
            "tcp46 0 0 *.22 *.* LISTEN".to_string(),
        ];
        let listeners = parse_macos_listeners(&lines);
        assert_eq!(listeners.len(), 3);
        assert_eq!(listeners[0].protocol, "TCP4");
        assert_eq!(listeners[0].port, 631);
        assert_eq!(listeners[1].protocol, "TCP6");
        assert_eq!(listeners[1].address, "::1");
        assert_eq!(listeners[2].protocol, "TCP46");
    }

    #[test]
    fn macos_same_port_ipv4_and_ipv6_bindings_remain_distinct() {
        let lines = vec![
            "tcp4 0 0 *.5000 *.* LISTEN".to_string(),
            "tcp6 0 0 *.5000 *.* LISTEN".to_string(),
        ];
        let listeners = parse_macos_listeners(&lines);
        assert_eq!(listeners.len(), 2);
        assert_eq!(listeners[0].protocol, "TCP4");
        assert_eq!(listeners[1].protocol, "TCP6");
        assert_eq!(listeners[0].port, listeners[1].port);
    }
}
