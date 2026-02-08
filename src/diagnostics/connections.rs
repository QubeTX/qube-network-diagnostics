use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct ConnectionEntry {
    pub protocol: String,
    pub local_addr: String,
    pub remote_addr: String,
    pub state: String,
    pub pid: Option<u32>,
    pub process_name: Option<String>,
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
async fn collect_windows() -> Option<Vec<ConnectionEntry>> {
    use sysinfo::System;

    let output = tokio::process::Command::new("netstat")
        .args(["-ano"])
        .output()
        .await
        .ok()?;

    let text = String::from_utf8_lossy(&output.stdout);
    let mut entries = Vec::new();

    let mut sys = System::new();
    sys.refresh_processes(sysinfo::ProcessesToUpdate::All, true);

    for line in text.lines() {
        let line = line.trim();
        let parts: Vec<&str> = line.split_whitespace().collect();

        if parts.len() >= 4 && (parts[0] == "TCP" || parts[0] == "UDP") {
            let pid = parts.last().and_then(|s| s.parse::<u32>().ok());
            let state = if parts[0] == "TCP" && parts.len() >= 5 {
                parts[3].to_string()
            } else {
                String::new()
            };

            let process_name = pid.and_then(|p| {
                sys.process(sysinfo::Pid::from_u32(p))
                    .map(|pr| pr.name().to_string_lossy().to_string())
            });

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

    Some(entries)
}

#[cfg(target_os = "macos")]
async fn collect_macos() -> Option<Vec<ConnectionEntry>> {
    let output = tokio::process::Command::new("netstat")
        .args(["-anp", "tcp"])
        .output()
        .await
        .ok()?;

    let text = String::from_utf8_lossy(&output.stdout);
    let mut entries = Vec::new();

    for line in text.lines() {
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

    Some(entries)
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
