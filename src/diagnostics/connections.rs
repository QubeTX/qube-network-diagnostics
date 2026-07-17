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
        if let Some(ref lines) = cache.macos_lsof_tcp {
            let entries = parse_macos_lsof_connections(lines);
            if !entries.is_empty() {
                return Some(entries);
            }
        }
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

    Some(parse_windows_connections(&lines, &process_map))
}

#[cfg(any(target_os = "macos", test))]
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

#[cfg(any(target_os = "macos", test))]
#[derive(Default)]
struct MacosLsofTcpRecord {
    name: Option<String>,
    state: Option<String>,
}

#[cfg(any(target_os = "macos", test))]
fn push_macos_lsof_record(
    entries: &mut Vec<ConnectionEntry>,
    seen: &mut std::collections::HashSet<(String, String, String, Option<u32>)>,
    pid: Option<u32>,
    process_name: &Option<String>,
    record: &mut MacosLsofTcpRecord,
) {
    let Some(name) = record.name.take() else {
        record.state = None;
        return;
    };
    let (local_addr, remote_addr) = name
        .split_once("->")
        .map(|(local, remote)| (local.to_string(), remote.to_string()))
        .unwrap_or_else(|| (name, "*:*".to_string()));
    let state = record.state.take().unwrap_or_default();
    let key = (local_addr.clone(), remote_addr.clone(), state.clone(), pid);
    if seen.insert(key) {
        entries.push(ConnectionEntry {
            protocol: "TCP".to_string(),
            local_addr,
            remote_addr,
            state,
            pid,
            process_name: process_name.clone(),
        });
    }
}

/// Parse `lsof -nP -iTCP -FpcPnT`. A `p` line starts a process, `f` starts a
/// socket record, `n` carries the complete endpoint, and `TST=` carries state.
/// This is the macOS 26 fallback for `netstat`'s fixed-width IPv6 truncation.
#[cfg(any(target_os = "macos", test))]
fn parse_macos_lsof_connections(lines: &[String]) -> Vec<ConnectionEntry> {
    let mut entries = Vec::new();
    let mut seen = std::collections::HashSet::new();
    let mut pid = None;
    let mut process_name = None;
    let mut record = MacosLsofTcpRecord::default();

    for line in lines {
        if let Some(value) = line.strip_prefix('p') {
            push_macos_lsof_record(&mut entries, &mut seen, pid, &process_name, &mut record);
            pid = value.parse().ok();
            process_name = None;
        } else if let Some(value) = line.strip_prefix('c') {
            process_name = Some(value.to_string());
        } else if line.starts_with('f') {
            push_macos_lsof_record(&mut entries, &mut seen, pid, &process_name, &mut record);
        } else if let Some(value) = line.strip_prefix('n') {
            record.name = Some(value.to_string());
        } else if let Some(value) = line.strip_prefix("TST=") {
            record.state = Some(value.to_string());
        }
    }
    push_macos_lsof_record(&mut entries, &mut seen, pid, &process_name, &mut record);

    entries
}

#[cfg(target_os = "macos")]
async fn collect_macos() -> Option<Vec<ConnectionEntry>> {
    if let Some(lines) = super::shared_cache::fetch_macos_lsof_tcp_lines().await {
        let entries = parse_macos_lsof_connections(&lines);
        if !entries.is_empty() {
            return Some(entries);
        }
    }

    let mut cmd = tokio::process::Command::new("netstat");
    cmd.args(["-anW", "-p", "tcp"]);
    let output = super::util::run_with_timeout(cmd, super::util::QUICK).await?;

    let text = String::from_utf8_lossy(&output.stdout);
    let lines: Vec<String> = text.lines().map(|l| l.to_string()).collect();
    Some(parse_macos_connections(&lines))
}

#[cfg(target_os = "linux")]
async fn collect_linux() -> Option<Vec<ConnectionEntry>> {
    let mut cmd = tokio::process::Command::new("ss");
    cmd.args(["-tupn"]);
    let output = super::util::run_with_timeout(cmd, super::util::QUICK).await?;

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn macos_wide_ipv6_endpoints_are_preserved() {
        let lines = vec![
            "tcp6 0 0 fe80::26fd:c41b:b5bc:ff7%utun12.1024 fe80::91b5:8f5b:1234%utun12.1024 SYN_SENT"
                .to_string(),
        ];
        let entries = parse_macos_connections(&lines);
        assert_eq!(entries.len(), 1);
        assert_eq!(
            entries[0].local_addr,
            "fe80::26fd:c41b:b5bc:ff7%utun12.1024"
        );
        assert_eq!(entries[0].state, "SYN_SENT");
    }

    #[test]
    fn macos_26_lsof_field_output_preserves_complete_ipv6_and_process() {
        // Captured-format fixture from macOS 26 `/usr/sbin/lsof -nP -iTCP
        // -FpcPnT`; addresses are intentionally long enough to be truncated
        // by Darwin `netstat`.
        let fixture = [
            "p7850",
            "cidentityservicesd",
            "f9",
            "PTCP",
            "n[fe80:1b::26fd:c41b:b5bc:ff7]:1024->[fe80:1b::91b5:8f5b:be19:6cf3]:1024",
            "TST=ESTABLISHED",
            "TQR=0",
            "TQS=0",
            "f10",
            "PTCP",
            "n*:51140",
            "TST=LISTEN",
        ]
        .map(str::to_string);

        let entries = parse_macos_lsof_connections(&fixture);
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].local_addr, "[fe80:1b::26fd:c41b:b5bc:ff7]:1024");
        assert_eq!(
            entries[0].remote_addr,
            "[fe80:1b::91b5:8f5b:be19:6cf3]:1024"
        );
        assert_eq!(entries[0].pid, Some(7850));
        assert_eq!(
            entries[0].process_name.as_deref(),
            Some("identityservicesd")
        );
        assert_eq!(entries[1].remote_addr, "*:*");
        assert_eq!(entries[1].state, "LISTEN");
    }

    #[test]
    fn macos_lsof_duplicate_socket_records_are_deduplicated() {
        let fixture = [
            "p42",
            "cdaemon",
            "f9",
            "PTCP",
            "n[fd00::1]:5000->[fd00::2]:443",
            "TST=ESTABLISHED",
            "f10",
            "PTCP",
            "n[fd00::1]:5000->[fd00::2]:443",
            "TST=ESTABLISHED",
        ]
        .map(str::to_string);
        assert_eq!(parse_macos_lsof_connections(&fixture).len(), 1);
    }
}
