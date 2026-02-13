use crate::config::Config;
use crate::diagnostics::{DiagnosticResults, DiagnosticStatus, TechnicianResults};
use crate::render::color::{colorize_status, dim};
use crate::render::table::ReportBuilder;

pub fn render(results: &DiagnosticResults, config: &Config) -> String {
    let label_width = 16;
    let data_width = 40;
    let chars = config.box_chars();

    let mut output = String::new();

    // == SUMMARY SECTION (same as user mode) ==
    let mut builder = ReportBuilder::new(label_width, data_width, chars)
        .header(config.title(), config.subtitle());

    builder = builder
        .span_row(&format!("  DIAGNOSTIC SUMMARY"))
        .divider();

    builder = render_summary_row(builder, &results.adapters, config);
    builder = render_summary_row(builder, &results.interfaces, config);
    builder = render_summary_row(builder, &results.gateway, config);
    builder = render_summary_row(builder, &results.dns, config);
    builder = render_summary_row(builder, &results.public_ip, config);
    builder = render_summary_row(builder, &results.latency, config);
    builder = render_summary_row(builder, &results.speed, config);
    builder = render_summary_row(builder, &results.ports, config);

    let (fail_count, warn_count) = count_issues(results);
    let overall = format_overall(fail_count, warn_count, config);
    builder = builder.divider();
    builder = builder.span_row(&format!("  OVERALL: {}", overall));

    if fail_count > 0 {
        builder = builder.span_row(&dim("  Run 'nd300 -f' to attempt automatic fixes", config));
    }

    output.push_str(&builder.finish());
    output.push('\n');

    // == DETAILED TECHNICAL SECTIONS ==

    // Interface details
    if let Some(ref ifaces) = results.interface_details {
        let mut b = ReportBuilder::new(label_width, data_width, chars);
        b = b.full_top_border().span_row("  NETWORK INTERFACES").divider();

        for (i, iface) in ifaces.iter().enumerate() {
            if i > 0 {
                b = b.divider();
            }
            b = b.row("Name", &iface.name);
            b = b.row("Type", &iface.interface_type);
            b = b.row("MAC", &iface.mac);
            b = b.row("Status", if iface.is_up { "Up" } else { "Down" });
            for ip in &iface.ip_addresses {
                b = b.row("IP Address", ip);
            }
        }
        output.push_str(&b.finish());
        output.push('\n');
    }

    // Adapter details
    if let Some(ref adapters) = results.adapter_details {
        if !adapters.is_empty() {
            let mut b = ReportBuilder::new(label_width, data_width, chars);
            b = b.full_top_border().span_row("  NETWORK ADAPTERS").divider();

            for (i, adapter) in adapters.iter().enumerate() {
                if i > 0 {
                    b = b.divider();
                }
                // Use description (hardware chip name) as primary label if available
                let adapter_label = adapter.description.as_deref().unwrap_or(&adapter.name);
                b = b.row("Adapter", adapter_label);

                // Show display type with physical medium detail
                let type_detail = if let Some(ref pm) = adapter.physical_medium {
                    format!("{} ({})", adapter.adapter_type, pm)
                } else {
                    adapter.adapter_type.clone()
                };
                b = b.row("Type", &type_detail);
                b = b.row("Status", &adapter.status);

                if let Some(ref mac) = adapter.mac_address {
                    b = b.row("MAC", mac);
                }

                // Link speed (show TX/RX if different, single value if same)
                match (adapter.link_speed_mbps, adapter.rx_link_speed_mbps) {
                    (Some(tx), Some(rx)) if tx == rx => {
                        b = b.row("Link Speed", &format_link_speed(tx));
                    }
                    (Some(tx), Some(rx)) => {
                        b = b.row("Link Speed", &format!("TX: {} / RX: {}", format_link_speed(tx), format_link_speed(rx)));
                    }
                    (Some(tx), None) => {
                        b = b.row("Link Speed", &format_link_speed(tx));
                    }
                    _ => {}
                }

                if let Some(ref gws) = adapter.gateways {
                    for gw in gws {
                        b = b.row("Gateway", gw);
                    }
                }
                if let Some(ref dns) = adapter.dns_servers {
                    for server in dns {
                        b = b.row("DNS", server);
                    }
                }
                if let Some(mtu) = adapter.mtu {
                    b = b.row("MTU", &mtu.to_string());
                }
                if let Some(metric) = adapter.ipv4_metric {
                    b = b.row("IPv4 Metric", &metric.to_string());
                }

                if let Some(ref drv) = adapter.driver_name {
                    b = b.row("Driver", drv);
                }
                if let Some(ref ver) = adapter.driver_version {
                    b = b.row("Driver Version", ver);
                }
                if let Some(ref date) = adapter.driver_date {
                    b = b.row("Driver Date", date);
                }
            }
            output.push_str(&b.finish());
            output.push('\n');
        }
    }

    // Gateway details
    if let Some(ref gw) = results.gateway_details {
        let mut b = ReportBuilder::new(label_width, data_width, chars);
        b = b.full_top_border().span_row("  GATEWAY").divider();
        b = b.row("IP Address", &gw.ip);
        b = b.row("Reachable", if gw.reachable { "Yes" } else { "No" });
        if let Some(lat) = gw.latency_ms {
            b = b.row("Latency", &format!("{:.1}ms", lat));
        }
        output.push_str(&b.finish());
        output.push('\n');
    }

    // DNS details
    if let Some(ref dns) = results.dns_details {
        let mut b = ReportBuilder::new(label_width, data_width, chars);
        b = b.full_top_border().span_row("  DNS SERVERS").divider();

        for server in &dns.servers {
            b = b.row("Server", &server.address);
        }

        if let Some(ref test) = dns.resolution_test {
            b = b.divider();
            b = b.row("Test Domain", &test.domain);
            b = b.row("Resolved", if test.resolved { "Yes" } else { "No" });
            b = b.row("Time", &format!("{:.1}ms", test.resolution_time_ms));
            for ip in &test.resolved_ips {
                b = b.row("Resolved IP", ip);
            }
        }
        output.push_str(&b.finish());
        output.push('\n');
    }

    // Public IP details
    if let Some(ref pip) = results.public_ip_details {
        let mut b = ReportBuilder::new(label_width, data_width, chars);
        b = b.full_top_border().span_row("  PUBLIC IP & GEOLOCATION").divider();
        b = b.row("Public IP", &pip.ip);
        b = b.row("Lookup Time", &format!("{:.0}ms", pip.lookup_time_ms));
        b = b.row("Behind NAT", if pip.behind_nat { "Yes" } else { "No" });

        if let Some(ref city) = pip.city {
            b = b.row("City", city);
        }
        if let Some(ref region) = pip.region {
            b = b.row("Region", region);
        }
        if let Some(ref country) = pip.country {
            b = b.row("Country", country);
        }
        if let Some(ref isp) = pip.isp {
            b = b.row("ISP", isp);
        }
        if let Some(ref org) = pip.org {
            b = b.row("Organization", org);
        }
        output.push_str(&b.finish());
        output.push('\n');
    }

    // Latency details
    if let Some(ref latencies) = results.latency_details {
        let mut b = ReportBuilder::new(label_width, data_width, chars);
        b = b.full_top_border().span_row("  LATENCY TESTS").divider();

        for (i, lat) in latencies.iter().enumerate() {
            if i > 0 {
                b = b.divider();
            }
            b = b.row("Host", &format!("{} ({})", lat.host, lat.label));
            b = b.row("Reachable", if lat.reachable { "Yes" } else { "No" });
            if let Some(min) = lat.min_ms {
                b = b.row("Min", &format!("{:.1}ms", min));
            }
            if let Some(avg) = lat.avg_ms {
                b = b.row("Avg", &format!("{:.1}ms", avg));
            }
            if let Some(max) = lat.max_ms {
                b = b.row("Max", &format!("{:.1}ms", max));
            }
            if let Some(jitter) = lat.jitter_ms {
                b = b.row("Jitter", &format!("{:.1}ms", jitter));
            }
            b = b.row("Packet Loss", &format!("{:.0}%", lat.packet_loss));
        }
        output.push_str(&b.finish());
        output.push('\n');
    }

    // Speed test details
    if let Some(ref speed) = results.speed_details {
        let mut b = ReportBuilder::new(label_width, data_width, chars);
        b = b.full_top_border().span_row("  SPEED TEST").divider();
        b = b.row("Server", &speed.server);
        b = b.row("Download", &format!("{:.1} Mbps", speed.download_mbps));
        b = b.row("Upload", &format!("{:.1} Mbps", speed.upload_mbps));
        b = b.row("DL Duration", &format!("{:.1}s", speed.download_duration_s));
        b = b.row("UL Duration", &format!("{:.1}s", speed.upload_duration_s));
        b = b.row("DL Data", &format_bytes(speed.download_bytes));
        b = b.row("UL Data", &format_bytes(speed.upload_bytes));
        output.push_str(&b.finish());
        output.push('\n');
    }

    // Port details
    if let Some(ref ports) = results.port_details {
        let mut b = ReportBuilder::new(label_width, data_width, chars);
        b = b.full_top_border().span_row("  PORT CONNECTIVITY").divider();

        for port in ports {
            let status = if port.open { "Open" } else { "Blocked" };
            let lat = port
                .latency_ms
                .map(|l| format!(" ({:.0}ms)", l))
                .unwrap_or_default();
            b = b.row(
                &format!("{} ({})", port.service, port.port),
                &format!("{}{}", status, lat),
            );
        }
        output.push_str(&b.finish());
        output.push('\n');
    }

    // == TECHNICIAN DEEP DIAGNOSTICS ==
    if let Some(ref tech) = results.technician {
        output.push_str(&render_technician_details(tech, config));
    }

    // Timestamp footer
    output.push_str(&format!("  Report generated: {}\n", results.timestamp));
    output.push('\n');

    output
}

fn render_technician_details(tech: &TechnicianResults, config: &Config) -> String {
    let label_width = 16;
    let data_width = 40;
    let chars = config.box_chars();
    let mut output = String::new();

    // ARP Table
    if let Some(ref arp) = tech.arp_table {
        if !arp.is_empty() {
            let mut b = ReportBuilder::new(label_width, data_width, chars);
            b = b.full_top_border().span_row("  ARP TABLE").divider();
            for entry in arp.iter().take(30) {
                b = b.row(&entry.ip, &format!("{} ({})", entry.mac, entry.entry_type));
            }
            if arp.len() > 30 {
                b = b.row("", &format!("... and {} more", arp.len() - 30));
            }
            output.push_str(&b.finish());
            output.push('\n');
        }
    }

    // Routing Table
    if let Some(ref routes) = tech.routing_table {
        if !routes.is_empty() {
            let mut b = ReportBuilder::new(label_width, data_width, chars);
            b = b.full_top_border().span_row("  ROUTING TABLE").divider();
            for route in routes.iter().take(20) {
                let gw = if route.gateway.is_empty() {
                    "direct".to_string()
                } else {
                    route.gateway.clone()
                };
                let metric = route.metric.map(|m| format!(" m:{}", m)).unwrap_or_default();
                b = b.row(&route.destination, &format!("via {} dev {}{}", gw, route.interface, metric));
            }
            if routes.len() > 20 {
                b = b.row("", &format!("... and {} more", routes.len() - 20));
            }
            output.push_str(&b.finish());
            output.push('\n');
        }
    }

    // Active Connections
    if let Some(ref conns) = tech.active_connections {
        if !conns.is_empty() {
            let established: Vec<_> = conns.iter().filter(|c| c.state == "ESTABLISHED").take(20).collect();
            if !established.is_empty() {
                let mut b = ReportBuilder::new(label_width, data_width, chars);
                b = b.full_top_border().span_row("  ACTIVE CONNECTIONS (ESTABLISHED)").divider();
                for conn in &established {
                    let proc_info = conn.process_name.as_deref().unwrap_or("?");
                    b = b.row(&conn.local_addr, &format!("{} [{}]", conn.remote_addr, proc_info));
                }
                let total_established = conns.iter().filter(|c| c.state == "ESTABLISHED").count();
                if total_established > 20 {
                    b = b.row("", &format!("... and {} more", total_established - 20));
                }
                output.push_str(&b.finish());
                output.push('\n');
            }
        }
    }

    // Listening Ports
    if let Some(ref ports) = tech.listening_ports {
        if !ports.is_empty() {
            let mut b = ReportBuilder::new(label_width, data_width, chars);
            b = b.full_top_border().span_row("  LISTENING PORTS").divider();
            for port in ports.iter().take(20) {
                let proc_info = port.process_name.as_deref().unwrap_or("?");
                b = b.row(
                    &format!("{} :{}", port.protocol, port.port),
                    &format!("{} [{}]", port.address, proc_info),
                );
            }
            if ports.len() > 20 {
                b = b.row("", &format!("... and {} more", ports.len() - 20));
            }
            output.push_str(&b.finish());
            output.push('\n');
        }
    }

    // DHCP Info
    if let Some(ref dhcp) = tech.dhcp_info {
        if !dhcp.is_empty() {
            let mut b = ReportBuilder::new(label_width, data_width, chars);
            b = b.full_top_border().span_row("  DHCP LEASES").divider();
            for (i, lease) in dhcp.iter().enumerate() {
                if i > 0 { b = b.divider(); }
                b = b.row("Interface", &lease.interface);
                b = b.row("DHCP Enabled", if lease.dhcp_enabled { "Yes" } else { "No" });
                if let Some(ref server) = lease.dhcp_server {
                    b = b.row("DHCP Server", server);
                }
                if let Some(ref ip) = lease.ip_address {
                    b = b.row("IP Address", ip);
                }
                if let Some(ref obtained) = lease.lease_obtained {
                    b = b.row("Obtained", obtained);
                }
                if let Some(ref expires) = lease.lease_expires {
                    b = b.row("Expires", expires);
                }
            }
            output.push_str(&b.finish());
            output.push('\n');
        }
    }

    // Protocol Statistics
    if let Some(ref stats) = tech.protocol_stats {
        let mut b = ReportBuilder::new(label_width, data_width, chars);
        b = b.full_top_border().span_row("  PROTOCOL STATISTICS").divider();

        b = b.row("TCP Active Opens", &stats.tcp.active_opens.to_string());
        b = b.row("TCP Passive Opens", &stats.tcp.passive_opens.to_string());
        b = b.row("TCP Current", &stats.tcp.current_connections.to_string());
        b = b.row("TCP Failed", &stats.tcp.failed_connections.to_string());
        b = b.row("TCP Resets", &stats.tcp.reset_connections.to_string());
        b = b.row("TCP Retransmits", &stats.tcp.segments_retransmitted.to_string());
        b = b.row("TCP Segments In", &stats.tcp.segments_received.to_string());
        b = b.row("TCP Segments Out", &stats.tcp.segments_sent.to_string());
        b = b.divider();
        b = b.row("UDP In", &stats.udp.datagrams_received.to_string());
        b = b.row("UDP Out", &stats.udp.datagrams_sent.to_string());
        b = b.row("UDP Errors", &stats.udp.receive_errors.to_string());
        b = b.divider();
        b = b.row("ICMP In", &stats.icmp.messages_received.to_string());
        b = b.row("ICMP Out", &stats.icmp.messages_sent.to_string());
        b = b.row("ICMP Errors In", &stats.icmp.errors_received.to_string());

        output.push_str(&b.finish());
        output.push('\n');
    }

    // Adapter HW Stats
    if let Some(ref hw) = tech.adapter_hw_stats {
        if !hw.is_empty() {
            let mut b = ReportBuilder::new(label_width, data_width, chars);
            b = b.full_top_border().span_row("  ADAPTER HARDWARE STATS").divider();
            for (i, stat) in hw.iter().enumerate() {
                if i > 0 { b = b.divider(); }
                b = b.row("Interface", &stat.name);
                b = b.row("RX Bytes", &format_bytes(stat.rx_bytes));
                b = b.row("TX Bytes", &format_bytes(stat.tx_bytes));
                b = b.row("RX Packets", &stat.rx_packets.to_string());
                b = b.row("TX Packets", &stat.tx_packets.to_string());
                b = b.row("RX Errors", &stat.rx_errors.to_string());
                b = b.row("TX Errors", &stat.tx_errors.to_string());
                if let Some(ref speed) = stat.link_speed {
                    b = b.row("Link Speed", speed);
                }
                if let Some(ref dup) = stat.duplex {
                    b = b.row("Duplex", dup);
                }
            }
            output.push_str(&b.finish());
            output.push('\n');
        }
    }

    // Proxy Config
    if let Some(ref proxy) = tech.proxy_config {
        let mut b = ReportBuilder::new(label_width, data_width, chars);
        b = b.full_top_border().span_row("  PROXY CONFIGURATION").divider();
        b = b.row("Proxy Enabled", if proxy.proxy_enabled { "Yes" } else { "No" });
        if let Some(ref http) = proxy.http_proxy {
            b = b.row("HTTP Proxy", http);
        }
        if let Some(ref https) = proxy.https_proxy {
            b = b.row("HTTPS Proxy", https);
        }
        if let Some(ref socks) = proxy.socks_proxy {
            b = b.row("SOCKS Proxy", socks);
        }
        if let Some(ref pac) = proxy.pac_url {
            b = b.row("PAC URL", pac);
        }
        if let Some(ref no) = proxy.no_proxy {
            b = b.row("No Proxy", no);
        }
        output.push_str(&b.finish());
        output.push('\n');
    }

    // VPN Detection
    if let Some(ref vpns) = tech.vpn_info {
        let mut b = ReportBuilder::new(label_width, data_width, chars);
        b = b.full_top_border().span_row("  VPN DETECTION").divider();
        for (i, vpn) in vpns.iter().enumerate() {
            if i > 0 { b = b.divider(); }
            b = b.row("VPN Adapter", &vpn.name);
            b = b.row("Type", &vpn.adapter_type);
            b = b.row("Status", &vpn.status);
            if let Some(ref vendor) = vpn.vendor {
                b = b.row("Vendor", vendor);
            }
            if vpn.is_enterprise {
                b = b.row("Policy", "Enterprise/Managed");
            }
            if let Some(ref iface) = vpn.interface_name {
                b = b.row("Interface", iface);
            }
            if let Some(ref ip) = vpn.ip_address {
                b = b.row("IP Address", ip);
            }
        }
        output.push_str(&b.finish());
        output.push('\n');
    }

    // Firewall Summary
    if let Some(ref fw) = tech.firewall_info {
        let mut b = ReportBuilder::new(label_width, data_width, chars);
        b = b.full_top_border().span_row("  FIREWALL STATUS").divider();
        b = b.row("Status", &fw.summary);
        for profile in &fw.profiles {
            b = b.row(
                &profile.name,
                if profile.enabled { "Enabled" } else { "Disabled" },
            );
        }
        output.push_str(&b.finish());
        output.push('\n');
    }

    // DNS Cache
    if let Some(ref cache) = tech.dns_cache {
        if !cache.is_empty() {
            let mut b = ReportBuilder::new(label_width, data_width, chars);
            b = b.full_top_border().span_row("  DNS CACHE").divider();
            for entry in cache.iter().take(20) {
                b = b.row(&entry.name, &format!("{}: {}", entry.record_type, entry.data));
            }
            if cache.len() > 20 {
                b = b.row("", &format!("... and {} more entries", cache.len() - 20));
            }
            output.push_str(&b.finish());
            output.push('\n');
        }
    }

    // IPv6 Status
    if let Some(ref ipv6) = tech.ipv6_info {
        let mut b = ReportBuilder::new(label_width, data_width, chars);
        b = b.full_top_border().span_row("  IPv6 STATUS").divider();
        b = b.row("Available", if ipv6.available { "Yes" } else { "No" });
        let conn_str = match ipv6.connectivity {
            crate::diagnostics::ipv6::Ipv6Connectivity::Full => "Full",
            crate::diagnostics::ipv6::Ipv6Connectivity::LinkLocal => "Link-local only",
            crate::diagnostics::ipv6::Ipv6Connectivity::None => "None",
        };
        b = b.row("Connectivity", conn_str);
        b = b.row("Dual Stack", if ipv6.dual_stack { "Yes" } else { "No" });

        for addr in ipv6.addresses.iter().take(10) {
            b = b.row(&addr.scope, &format!("{} ({})", addr.address, addr.interface));
        }
        output.push_str(&b.finish());
        output.push('\n');
    }

    // MTU Info
    if let Some(ref mtus) = tech.mtu_info {
        if !mtus.is_empty() {
            let mut b = ReportBuilder::new(label_width, data_width, chars);
            b = b.full_top_border().span_row("  MTU PER INTERFACE").divider();
            for mtu in mtus {
                b = b.row(&mtu.interface, &mtu.mtu.to_string());
            }
            output.push_str(&b.finish());
            output.push('\n');
        }
    }

    // Connection States
    if let Some(ref states) = tech.connection_states {
        let mut b = ReportBuilder::new(label_width, data_width, chars);
        b = b.full_top_border().span_row("  TCP CONNECTION STATES").divider();
        b = b.row("ESTABLISHED", &states.established.to_string());
        b = b.row("TIME_WAIT", &states.time_wait.to_string());
        b = b.row("CLOSE_WAIT", &states.close_wait.to_string());
        b = b.row("FIN_WAIT_1", &states.fin_wait_1.to_string());
        b = b.row("FIN_WAIT_2", &states.fin_wait_2.to_string());
        b = b.row("SYN_SENT", &states.syn_sent.to_string());
        b = b.row("SYN_RECEIVED", &states.syn_received.to_string());
        b = b.row("CLOSING", &states.closing.to_string());
        b = b.row("LAST_ACK", &states.last_ack.to_string());
        b = b.row("LISTEN", &states.listen.to_string());
        output.push_str(&b.finish());
        output.push('\n');
    }

    // Bufferbloat
    if let Some(ref bb) = tech.bufferbloat {
        let mut b = ReportBuilder::new(label_width, data_width, chars);
        b = b.full_top_border().span_row("  BUFFERBLOAT TEST").divider();
        b = b.row("Grade", &bb.grade);
        b = b.row("Unloaded Latency", &format!("{:.1}ms", bb.unloaded_latency_ms));
        if let Some(loaded) = bb.loaded_latency_ms {
            b = b.row("Loaded Latency", &format!("{:.1}ms", loaded));
        }
        if let Some(bloat) = bb.bloat_ms {
            b = b.row("Bloat", &format!("+{:.1}ms", bloat));
        }
        b = b.row("Assessment", &bb.description);
        output.push_str(&b.finish());
        output.push('\n');
    }

    // Reverse DNS
    if let Some(ref rdns) = tech.reverse_dns {
        if !rdns.is_empty() {
            let mut b = ReportBuilder::new(label_width, data_width, chars);
            b = b.full_top_border().span_row("  REVERSE DNS").divider();
            for entry in rdns {
                let hostname = entry.hostname.as_deref().unwrap_or("(no PTR)");
                b = b.row(&format!("{} ({})", entry.label, entry.ip), hostname);
            }
            output.push_str(&b.finish());
            output.push('\n');
        }
    }

    // TLS Inspection
    if let Some(ref tls) = tech.tls_inspection {
        let mut b = ReportBuilder::new(label_width, data_width, chars);
        b = b.full_top_border().span_row("  TLS INSPECTION CHECK").divider();
        b = b.row("Intercepted", if tls.detected { "DETECTED" } else { "No" });
        b = b.row("Assessment", &tls.description);
        output.push_str(&b.finish());
        output.push('\n');
    }

    // Traffic Counters
    if let Some(ref traffic) = tech.traffic_counters {
        if !traffic.is_empty() {
            let mut b = ReportBuilder::new(label_width, data_width, chars);
            b = b.full_top_border().span_row("  TRAFFIC COUNTERS (since boot)").divider();
            for counter in traffic {
                b = b.row(
                    &counter.interface,
                    &format!("RX {} / TX {}", counter.rx_formatted, counter.tx_formatted),
                );
            }
            output.push_str(&b.finish());
            output.push('\n');
        }
    }

    output
}

fn render_summary_row(
    builder: ReportBuilder,
    result: &crate::diagnostics::DiagnosticResult,
    config: &Config,
) -> ReportBuilder {
    let icon = config.status_chars(&result.status);
    let colored_icon = colorize_status(icon, &result.status, config);
    let label = format!("{} {}", colored_icon, result.category);

    let lines: Vec<&str> = result.summary.split('\n').collect();
    let mut b = builder.row(&label, lines[0]);
    for line in lines.iter().skip(1) {
        b = b.row("", line);
    }
    b
}

fn count_issues(results: &DiagnosticResults) -> (usize, usize) {
    let statuses = [
        &results.adapters.status,
        &results.interfaces.status,
        &results.gateway.status,
        &results.dns.status,
        &results.public_ip.status,
        &results.latency.status,
        &results.speed.status,
        &results.ports.status,
    ];

    let fails = statuses.iter().filter(|s| ***s == DiagnosticStatus::Fail).count();
    let warns = statuses.iter().filter(|s| ***s == DiagnosticStatus::Warn).count();
    (fails, warns)
}

fn format_overall(fails: usize, warns: usize, config: &Config) -> String {
    if fails > 0 && warns > 0 {
        let text = format!("{} failure{}, {} warning{}", fails, if fails > 1 { "s" } else { "" }, warns, if warns > 1 { "s" } else { "" });
        colorize_status(&text, &DiagnosticStatus::Fail, config)
    } else if fails > 0 {
        let text = format!("{} failure{} detected", fails, if fails > 1 { "s" } else { "" });
        colorize_status(&text, &DiagnosticStatus::Fail, config)
    } else if warns > 0 {
        let text = format!("{} warning{} detected", warns, if warns > 1 { "s" } else { "" });
        colorize_status(&text, &DiagnosticStatus::Warn, config)
    } else {
        colorize_status("All diagnostics passed", &DiagnosticStatus::Ok, config)
    }
}

fn format_link_speed(mbps: u64) -> String {
    if mbps >= 1000 {
        format!("{:.1} Gbps", mbps as f64 / 1000.0)
    } else {
        format!("{} Mbps", mbps)
    }
}

fn format_bytes(bytes: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = 1024 * KB;
    const GB: u64 = 1024 * MB;

    if bytes >= GB {
        format!("{:.2} GB", bytes as f64 / GB as f64)
    } else if bytes >= MB {
        format!("{:.1} MB", bytes as f64 / MB as f64)
    } else if bytes >= KB {
        format!("{:.1} KB", bytes as f64 / KB as f64)
    } else {
        format!("{} B", bytes)
    }
}
