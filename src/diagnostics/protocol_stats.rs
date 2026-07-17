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
    let mut cmd = tokio::process::Command::new("netstat");
    cmd.args(["-s"]);
    let output = super::util::run_with_timeout(cmd, super::util::QUICK).await?;

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
    let mut cmd = tokio::process::Command::new("netstat");
    cmd.args(["-s"]);
    let output = super::util::run_with_timeout(cmd, super::util::QUICK).await?;

    let text = String::from_utf8_lossy(&output.stdout);

    let mut stats = parse_macos_protocol_stats(&text)?;
    let mut connections_cmd = tokio::process::Command::new("netstat");
    connections_cmd.args(["-anW", "-p", "tcp"]);
    let connections = super::util::run_with_timeout(connections_cmd, super::util::QUICK).await?;
    stats.tcp.current_connections = String::from_utf8_lossy(&connections.stdout)
        .lines()
        .filter(|line| line.split_whitespace().last() == Some("ESTABLISHED"))
        .count() as u64;
    Some(stats)
}

#[cfg(any(target_os = "macos", test))]
fn parse_macos_protocol_stats(text: &str) -> Option<ProtocolStatistics> {
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
    let mut icmp_input_histogram = false;
    let mut icmp_input_total = 0u64;
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
            icmp_input_histogram = false;
            continue;
        }
        if trimmed.ends_with(':') && !trimmed.contains(' ') {
            section = "";
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
                } else if trimmed.ends_with("bad reset") {
                    tcp.reset_connections = val;
                } else if trimmed.contains("data packet") && trimmed.contains("retransmitted") {
                    tcp.segments_retransmitted = val;
                } else if trimmed.ends_with("packet sent") || trimmed.ends_with("packets sent") {
                    tcp.segments_sent = val;
                } else if trimmed.ends_with("packet received")
                    || trimmed.ends_with("packets received")
                {
                    tcp.segments_received = val;
                }
            }
            "udp" => {
                if trimmed.contains("datagram") && trimmed.contains("received") {
                    udp.datagrams_received = val;
                } else if trimmed.contains("datagram") && trimmed.contains("output") {
                    udp.datagrams_sent = val;
                } else if trimmed.contains("dropped due to no socket") {
                    udp.no_port_errors = val;
                } else if trimmed.contains("incomplete header")
                    || trimmed.contains("bad data length")
                    || trimmed.contains("bad checksum")
                    || trimmed.contains("full socket buffers")
                {
                    udp.receive_errors = udp.receive_errors.saturating_add(val);
                }
            }
            "icmp" => {
                if trimmed == "Input histogram:" {
                    icmp_input_histogram = true;
                } else if trimmed.ends_with("calls to icmp_error") {
                    icmp.errors_sent = val;
                    icmp.messages_sent = icmp.messages_sent.saturating_add(val);
                } else if trimmed.ends_with("message response generated") {
                    icmp.messages_sent = icmp.messages_sent.saturating_add(val);
                } else if icmp_input_histogram && trimmed.contains(':') {
                    let histogram_value = trimmed
                        .rsplit_once(':')
                        .and_then(|(_, value)| value.trim().parse::<u64>().ok())
                        .unwrap_or(0);
                    icmp_input_total = icmp_input_total.saturating_add(histogram_value);
                } else if trimmed.contains("bad code")
                    || trimmed.contains("bad checksum")
                    || trimmed.contains("bad length")
                    || trimmed.contains("minimum length")
                {
                    icmp.errors_received = icmp.errors_received.saturating_add(val);
                }
            }
            _ => {}
        }
    }
    icmp.messages_received = icmp_input_total;

    // Current macOS releases can expose a populated TCP connection table while
    // `netstat -s` returns an all-zero TCP block. The public schema uses plain
    // integers, so serializing that block would claim measured zeroes. Omit
    // this optional technician section until the kernel supplies counters.
    let tcp_available = tcp.active_opens > 0
        || tcp.passive_opens > 0
        || tcp.failed_connections > 0
        || tcp.reset_connections > 0
        || tcp.current_connections > 0
        || tcp.segments_received > 0
        || tcp.segments_sent > 0
        || tcp.segments_retransmitted > 0;
    tcp_available.then_some(ProtocolStatistics { tcp, udp, icmp })
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

#[cfg(any(target_os = "macos", test))]
fn extract_leading_number(line: &str) -> Option<u64> {
    let num_str: String = line.chars().take_while(|c| c.is_ascii_digit()).collect();
    num_str.parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn current_macos_udp_wording_and_icmp_histogram_parse() {
        let fixture = "tcp:\n\t12 packets sent\n\t3 data packets retransmitted\n\t15 packets received\n\t2 connection requests\n\t1 connection accept\nudp:\n\t409241 datagrams received\n\t\t7 dropped due to no socket\n\t\t3 dropped due to full socket buffers\n\t104267 datagrams output\nicmp:\n\t5 calls to icmp_error\n\tInput histogram:\n\t\techo reply: 8\n\t\tdestination unreachable: 2\n\t1 message response generated\n";
        let stats = parse_macos_protocol_stats(fixture).unwrap();
        assert_eq!(stats.tcp.segments_sent, 12);
        assert_eq!(stats.tcp.segments_retransmitted, 3);
        assert_eq!(stats.udp.datagrams_sent, 104267);
        assert_eq!(stats.udp.no_port_errors, 7);
        assert_eq!(stats.udp.receive_errors, 3);
        assert_eq!(stats.icmp.messages_received, 10);
        assert_eq!(stats.icmp.messages_sent, 6);
    }

    #[test]
    fn suppressed_macos_tcp_counters_omit_optional_section() {
        let fixture = "tcp:\n\t0 packet sent\n\t0 packet received\n\t0 connection request\n\t0 connection accept\nudp:\n\t12 datagrams received\n\t9 datagrams output\n";
        assert!(parse_macos_protocol_stats(fixture).is_none());
    }
}
