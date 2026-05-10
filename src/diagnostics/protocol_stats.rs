use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct ProtocolStatistics {
    pub tcp: TcpStats,
    pub udp: UdpStats,
    pub icmp: IcmpStats,
}

#[derive(Debug, Clone, Serialize)]
pub struct TcpStats {
    pub active_opens: u64,
    pub passive_opens: u64,
    pub failed_connections: u64,
    pub reset_connections: u64,
    pub current_connections: u64,
    pub segments_received: u64,
    pub segments_sent: u64,
    pub segments_retransmitted: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct UdpStats {
    pub datagrams_received: u64,
    pub datagrams_sent: u64,
    pub receive_errors: u64,
    pub no_port_errors: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct IcmpStats {
    pub messages_received: u64,
    pub messages_sent: u64,
    pub errors_received: u64,
    pub errors_sent: u64,
}

pub async fn collect() -> Option<ProtocolStatistics> {
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
async fn collect_windows() -> Option<ProtocolStatistics> {
    let output = tokio::process::Command::new("netstat")
        .args(["-s"])
        .output()
        .await
        .ok()?;

    let text = String::from_utf8_lossy(&output.stdout);
    let mut tcp = TcpStats {
        active_opens: 0,
        passive_opens: 0,
        failed_connections: 0,
        reset_connections: 0,
        current_connections: 0,
        segments_received: 0,
        segments_sent: 0,
        segments_retransmitted: 0,
    };
    let mut udp = UdpStats {
        datagrams_received: 0,
        datagrams_sent: 0,
        receive_errors: 0,
        no_port_errors: 0,
    };
    let mut icmp = IcmpStats {
        messages_received: 0,
        messages_sent: 0,
        errors_received: 0,
        errors_sent: 0,
    };

    let mut section = "";
    for line in text.lines() {
        let line = line.trim();
        if line.contains("TCP Statistics") {
            section = "tcp";
            continue;
        }
        if line.contains("UDP Statistics") {
            section = "udp";
            continue;
        }
        if line.contains("ICMPv4 Statistics") || line.contains("ICMP Statistics") {
            section = "icmp";
            continue;
        }
        if line.contains("IPv4 Statistics") || line.contains("IPv6 Statistics") {
            section = "";
            continue;
        }

        let val = extract_stat_value(line).unwrap_or(0);

        match section {
            "tcp" => {
                if line.contains("Active Opens") {
                    tcp.active_opens = val;
                } else if line.contains("Passive Opens") {
                    tcp.passive_opens = val;
                } else if line.contains("Failed") {
                    tcp.failed_connections = val;
                } else if line.contains("Reset") && line.contains("Connection") {
                    tcp.reset_connections = val;
                } else if line.contains("Current") {
                    tcp.current_connections = val;
                } else if line.contains("Segments Received") {
                    tcp.segments_received = val;
                } else if line.contains("Segments Sent") && !line.contains("Re") {
                    tcp.segments_sent = val;
                } else if line.contains("Retransmit") {
                    tcp.segments_retransmitted = val;
                }
            }
            "udp" => {
                if line.contains("Datagrams Received") {
                    udp.datagrams_received = val;
                } else if line.contains("No Ports") {
                    udp.no_port_errors = val;
                } else if line.contains("Receive Errors") {
                    udp.receive_errors = val;
                } else if line.contains("Datagrams Sent") {
                    udp.datagrams_sent = val;
                }
            }
            "icmp" => {
                if line.contains("Messages") && line.contains("Received") {
                    icmp.messages_received = val;
                } else if line.contains("Messages") && line.contains("Sent") {
                    icmp.messages_sent = val;
                } else if line.contains("Errors") && line.contains("Received") {
                    icmp.errors_received = val;
                } else if line.contains("Errors") && line.contains("Sent") {
                    icmp.errors_sent = val;
                }
            }
            _ => {}
        }
    }

    Some(ProtocolStatistics { tcp, udp, icmp })
}

#[cfg(target_os = "macos")]
async fn collect_macos() -> Option<ProtocolStatistics> {
    let output = tokio::process::Command::new("netstat")
        .args(["-s"])
        .output()
        .await
        .ok()?;

    let text = String::from_utf8_lossy(&output.stdout);

    // macOS netstat -s output parsing (similar structure to Windows)
    let mut tcp = TcpStats {
        active_opens: 0,
        passive_opens: 0,
        failed_connections: 0,
        reset_connections: 0,
        current_connections: 0,
        segments_received: 0,
        segments_sent: 0,
        segments_retransmitted: 0,
    };
    let mut udp = UdpStats {
        datagrams_received: 0,
        datagrams_sent: 0,
        receive_errors: 0,
        no_port_errors: 0,
    };
    let mut icmp = IcmpStats {
        messages_received: 0,
        messages_sent: 0,
        errors_received: 0,
        errors_sent: 0,
    };

    let mut section = "";
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed == "tcp:" {
            section = "tcp";
            continue;
        }
        if trimmed == "udp:" {
            section = "udp";
            continue;
        }
        if trimmed == "icmp:" {
            section = "icmp";
            continue;
        }

        let val = extract_leading_number(trimmed).unwrap_or(0);

        match section {
            "tcp" => {
                if trimmed.contains("connection request") {
                    tcp.active_opens = val;
                } else if trimmed.contains("connection accept") {
                    tcp.passive_opens = val;
                } else if trimmed.contains("bad connection") {
                    tcp.failed_connections = val;
                } else if trimmed.contains("reset") {
                    tcp.reset_connections = val;
                } else if trimmed.contains("packet") && trimmed.contains("sent") {
                    tcp.segments_sent = val;
                } else if trimmed.contains("packet") && trimmed.contains("received") {
                    tcp.segments_received = val;
                } else if trimmed.contains("retransmit") {
                    tcp.segments_retransmitted = val;
                }
            }
            "udp" => {
                if trimmed.contains("datagram") && trimmed.contains("received") {
                    udp.datagrams_received = val;
                } else if trimmed.contains("datagram") && trimmed.contains("sent") {
                    udp.datagrams_sent = val;
                }
            }
            "icmp" => {
                if trimmed.contains("response") && trimmed.contains("received") {
                    icmp.messages_received = val;
                } else if trimmed.contains("sent") {
                    icmp.messages_sent = val;
                }
            }
            _ => {}
        }
    }

    Some(ProtocolStatistics { tcp, udp, icmp })
}

#[cfg(target_os = "linux")]
async fn collect_linux() -> Option<ProtocolStatistics> {
    let mut tcp = TcpStats {
        active_opens: 0,
        passive_opens: 0,
        failed_connections: 0,
        reset_connections: 0,
        current_connections: 0,
        segments_received: 0,
        segments_sent: 0,
        segments_retransmitted: 0,
    };
    let mut udp = UdpStats {
        datagrams_received: 0,
        datagrams_sent: 0,
        receive_errors: 0,
        no_port_errors: 0,
    };
    let mut icmp = IcmpStats {
        messages_received: 0,
        messages_sent: 0,
        errors_received: 0,
        errors_sent: 0,
    };

    // Read /proc/net/snmp
    if let Ok(content) = tokio::fs::read_to_string("/proc/net/snmp").await {
        let lines: Vec<&str> = content.lines().collect();
        for i in (0..lines.len()).step_by(2) {
            if i + 1 >= lines.len() {
                break;
            }
            let headers: Vec<&str> = lines[i].split_whitespace().collect();
            let values: Vec<&str> = lines[i + 1].split_whitespace().collect();

            if headers.first() == Some(&"Tcp:") && headers.len() == values.len() {
                for (j, header) in headers.iter().enumerate() {
                    let val: u64 = values.get(j).and_then(|s| s.parse().ok()).unwrap_or(0);
                    match *header {
                        "ActiveOpens" => tcp.active_opens = val,
                        "PassiveOpens" => tcp.passive_opens = val,
                        "AttemptFails" => tcp.failed_connections = val,
                        "EstabResets" => tcp.reset_connections = val,
                        "CurrEstab" => tcp.current_connections = val,
                        "InSegs" => tcp.segments_received = val,
                        "OutSegs" => tcp.segments_sent = val,
                        "RetransSegs" => tcp.segments_retransmitted = val,
                        _ => {}
                    }
                }
            } else if headers.first() == Some(&"Udp:") && headers.len() == values.len() {
                for (j, header) in headers.iter().enumerate() {
                    let val: u64 = values.get(j).and_then(|s| s.parse().ok()).unwrap_or(0);
                    match *header {
                        "InDatagrams" => udp.datagrams_received = val,
                        "OutDatagrams" => udp.datagrams_sent = val,
                        "InErrors" => udp.receive_errors = val,
                        "NoPorts" => udp.no_port_errors = val,
                        _ => {}
                    }
                }
            } else if headers.first() == Some(&"Icmp:") && headers.len() == values.len() {
                for (j, header) in headers.iter().enumerate() {
                    let val: u64 = values.get(j).and_then(|s| s.parse().ok()).unwrap_or(0);
                    match *header {
                        "InMsgs" => icmp.messages_received = val,
                        "OutMsgs" => icmp.messages_sent = val,
                        "InErrors" => icmp.errors_received = val,
                        "OutErrors" => icmp.errors_sent = val,
                        _ => {}
                    }
                }
            }
        }
    }

    Some(ProtocolStatistics { tcp, udp, icmp })
}

#[cfg(windows)]
fn extract_stat_value(line: &str) -> Option<u64> {
    // "  Active Opens              = 12345"
    if let Some(pos) = line.find('=') {
        let after = line[pos + 1..].trim();
        return after.parse().ok();
    }
    None
}

#[cfg(target_os = "macos")]
fn extract_leading_number(line: &str) -> Option<u64> {
    let num_str: String = line.chars().take_while(|c| c.is_ascii_digit()).collect();
    num_str.parse().ok()
}
