pub mod adapter_hw_stats;
pub mod adapters;
pub mod arp;
pub mod bufferbloat;
pub mod connection_states;
pub mod connections;
pub mod dhcp;
pub mod dns;
pub mod dns_cache;
pub mod firewall;
pub mod gateway;
pub mod interfaces;
pub mod ipv6;
pub mod latency;
pub mod listening_ports;
pub mod mtu;
pub mod ping;
pub mod ports;
pub mod protocol_stats;
pub mod proxy;
pub mod public_ip;
pub mod reverse_dns;
pub mod routing_table;
pub mod shared_cache;
pub mod speed;
pub mod tls_inspection;
pub mod traffic_counters;
pub mod util;
pub mod vpn;

use serde::Serialize;

use crate::config::Config;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub enum DiagnosticStatus {
    Ok,
    Warn,
    Fail,
    Skip,
}

#[derive(Debug, Clone, Serialize)]
pub struct DiagnosticResult {
    pub category: String,
    pub status: DiagnosticStatus,
    pub summary: String,
    pub details: Option<String>,
    /// True when this row was fabricated because the check did not finish
    /// before the wall-clock cap — it is evidence of overall slowness, not of
    /// this specific subsystem failing. The fix flow's triage skips these
    /// rows; the exit code still treats them as Fail. (Additive, v3.4.0+.)
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub timed_out: bool,
}

impl DiagnosticResult {
    pub fn ok(category: impl Into<String>, summary: impl Into<String>) -> Self {
        Self {
            category: category.into(),
            status: DiagnosticStatus::Ok,
            summary: summary.into(),
            details: None,
            timed_out: false,
        }
    }

    pub fn warn(category: impl Into<String>, summary: impl Into<String>) -> Self {
        Self {
            category: category.into(),
            status: DiagnosticStatus::Warn,
            summary: summary.into(),
            details: None,
            timed_out: false,
        }
    }

    pub fn fail(category: impl Into<String>, summary: impl Into<String>) -> Self {
        Self {
            category: category.into(),
            status: DiagnosticStatus::Fail,
            summary: summary.into(),
            details: None,
            timed_out: false,
        }
    }

    pub fn skip(category: impl Into<String>, summary: impl Into<String>) -> Self {
        Self {
            category: category.into(),
            status: DiagnosticStatus::Skip,
            summary: summary.into(),
            details: None,
            timed_out: false,
        }
    }

    /// A fabricated Fail row for a check that did not finish before the
    /// wall-clock cap. Marked `timed_out` so consumers (the fix flow's
    /// triage, JSON scripters) can tell it apart from a real finding.
    pub fn timed_out_fail(category: impl Into<String>) -> Self {
        Self {
            category: category.into(),
            status: DiagnosticStatus::Fail,
            summary: "Timed out — network severely degraded".to_string(),
            details: None,
            timed_out: true,
        }
    }

    pub fn with_details(mut self, details: impl Into<String>) -> Self {
        self.details = Some(details.into());
        self
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct DiagnosticResults {
    pub timestamp: String,
    pub adapters: DiagnosticResult,
    pub interfaces: DiagnosticResult,
    pub gateway: DiagnosticResult,
    pub dns: DiagnosticResult,
    pub public_ip: DiagnosticResult,
    pub latency: DiagnosticResult,
    pub speed: DiagnosticResult,
    pub ports: DiagnosticResult,

    // Detailed data for rendering
    #[serde(skip_serializing_if = "Option::is_none")]
    pub interface_details: Option<Vec<interfaces::InterfaceInfo>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub adapter_details: Option<Vec<adapters::AdapterInfo>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gateway_details: Option<gateway::GatewayInfo>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dns_details: Option<dns::DnsInfo>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub public_ip_details: Option<public_ip::PublicIpInfo>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub latency_details: Option<Vec<latency::LatencyResult>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub speed_details: Option<crate::speedtest::SpeedTestResult>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub port_details: Option<Vec<ports::PortResult>>,

    // Technician-mode deep diagnostics
    #[serde(skip_serializing_if = "Option::is_none")]
    pub technician: Option<TechnicianResults>,

    /// True when the wall-clock cap fired mid-run: the rows above are partial
    /// (completed checks are real; unfinished ones carry fabricated
    /// `timed_out` Fail rows). (Additive, v3.4.0+.)
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub timed_out: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct TechnicianResults {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub arp_table: Option<Vec<arp::ArpEntry>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub routing_table: Option<Vec<routing_table::RouteEntry>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub active_connections: Option<Vec<connections::ConnectionEntry>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub listening_ports: Option<Vec<listening_ports::ListeningPort>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dhcp_info: Option<Vec<dhcp::DhcpLease>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub protocol_stats: Option<protocol_stats::ProtocolStatistics>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub adapter_hw_stats: Option<Vec<adapter_hw_stats::AdapterHwStat>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub proxy_config: Option<proxy::ProxyConfig>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub vpn_info: Option<Vec<vpn::VpnAdapter>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub firewall_info: Option<firewall::FirewallInfo>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dns_cache: Option<Vec<dns_cache::DnsCacheEntry>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ipv6_info: Option<ipv6::Ipv6Info>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mtu_info: Option<Vec<mtu::MtuInfo>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub connection_states: Option<connection_states::ConnectionStates>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bufferbloat: Option<bufferbloat::BufferbloatResult>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reverse_dns: Option<Vec<reverse_dns::ReverseDnsEntry>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tls_inspection: Option<tls_inspection::TlsInspectionResult>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub traffic_counters: Option<Vec<traffic_counters::TrafficCounter>>,
}

/// Wall-clock cap for a full `run_all` pass.
///
/// Diagnostics shell out and resolve hostnames, which on a badly-broken
/// network could still take longer than is useful even though each individual
/// call is bounded. The cap must scale with `--speed-duration`: the
/// diagnostic speed test runs the CF + NDT7 providers sequentially, each
/// doing a download AND an upload for `speed_duration` seconds per direction,
/// so the legitimate speed-test floor is ~`4 * speed_duration` (2 providers ×
/// 2 directions). A fixed 90s would falsely truncate a deliberately long
/// speed test (e.g. `--speed-duration 60`). 30s of headroom covers
/// discovery/latency probes and the non-speed diagnostics; a 90s floor keeps
/// the default/`--fast` behavior unchanged.
pub fn run_all_cap(config: &Config) -> std::time::Duration {
    const RUN_ALL_CAP_FLOOR: std::time::Duration = std::time::Duration::from_secs(90);
    const RUN_ALL_CAP_HEADROOM: u64 = 30;

    if config.skip_speed {
        RUN_ALL_CAP_FLOOR
    } else {
        std::time::Duration::from_secs(4 * config.speed_duration + RUN_ALL_CAP_HEADROOM)
            .max(RUN_ALL_CAP_FLOOR)
    }
}

pub async fn run_all(config: &Config, cap: std::time::Duration) -> DiagnosticResults {
    let deadline = tokio::time::Instant::now() + cap;
    let timestamp = chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string();
    let mut timed_out = false;

    // Spawn the core checks so that, if the deadline fires, the ones that
    // already finished can still be harvested — the user gets partial results
    // instead of nothing.
    let mut adapters_h = tokio::spawn(adapters::check());
    let mut interfaces_h = tokio::spawn(interfaces::check());
    let mut gateway_h = tokio::spawn(gateway::check());
    let mut dns_h = tokio::spawn(dns::check());
    let mut public_ip_h = tokio::spawn(public_ip::check());
    let mut latency_h = tokio::spawn(latency::check());
    let mut ports_h = tokio::spawn(ports::check());

    // Race the joined set against the deadline. `&mut JoinHandle` is a future
    // (JoinHandle: Unpin), so on the deadline branch the handles are still
    // owned and can be harvested individually below.
    let completed = tokio::select! {
        results = async {
            tokio::join!(
                &mut adapters_h,
                &mut interfaces_h,
                &mut gateway_h,
                &mut dns_h,
                &mut public_ip_h,
                &mut latency_h,
                &mut ports_h,
            )
        } => Some(results),
        _ = tokio::time::sleep_until(deadline) => None,
    };

    let (
        (adapters_result, adapter_details),
        (interfaces_result, interface_details),
        (gateway_result, gateway_details),
        (dns_result, dns_details),
        (public_ip_result, public_ip_details),
        (latency_result, latency_details),
        (ports_result, port_details),
    ) = match completed {
        Some((a, i, g, d, p, l, po)) => (
            // A JoinError here means the check panicked; surface it as a
            // timed-out-style fabricated row rather than crashing the run.
            a.unwrap_or_else(|_| (DiagnosticResult::timed_out_fail("Adapters"), Vec::new())),
            i.unwrap_or_else(|_| (DiagnosticResult::timed_out_fail("Network"), Vec::new())),
            g.unwrap_or_else(|_| (DiagnosticResult::timed_out_fail("Gateway"), None)),
            d.unwrap_or_else(|_| (DiagnosticResult::timed_out_fail("DNS"), None)),
            p.unwrap_or_else(|_| (DiagnosticResult::timed_out_fail("Internet"), None)),
            l.unwrap_or_else(|_| (DiagnosticResult::timed_out_fail("Latency"), Vec::new())),
            po.unwrap_or_else(|_| (DiagnosticResult::timed_out_fail("Ports"), Vec::new())),
        ),
        None => {
            timed_out = true;
            (
                util::harvest_or(
                    adapters_h,
                    (DiagnosticResult::timed_out_fail("Adapters"), Vec::new()),
                )
                .await,
                util::harvest_or(
                    interfaces_h,
                    (DiagnosticResult::timed_out_fail("Network"), Vec::new()),
                )
                .await,
                util::harvest_or(
                    gateway_h,
                    (DiagnosticResult::timed_out_fail("Gateway"), None),
                )
                .await,
                util::harvest_or(dns_h, (DiagnosticResult::timed_out_fail("DNS"), None)).await,
                util::harvest_or(
                    public_ip_h,
                    (DiagnosticResult::timed_out_fail("Internet"), None),
                )
                .await,
                util::harvest_or(
                    latency_h,
                    (DiagnosticResult::timed_out_fail("Latency"), Vec::new()),
                )
                .await,
                util::harvest_or(
                    ports_h,
                    (DiagnosticResult::timed_out_fail("Ports"), Vec::new()),
                )
                .await,
            )
        }
    };

    // Run speed test sequentially (it needs to saturate bandwidth), bounded
    // by whatever budget remains.
    let now = tokio::time::Instant::now();
    let (speed_result, speed_details) = if config.skip_speed {
        (
            DiagnosticResult::skip("Speed", "Speed test skipped (--fast)"),
            None,
        )
    } else if timed_out || now >= deadline {
        timed_out = true;
        (
            DiagnosticResult::skip("Speed", "Skipped — diagnostics timed out"),
            None,
        )
    } else {
        match tokio::time::timeout_at(deadline, speed::check(config)).await {
            Ok(outcome) => outcome,
            Err(_) => {
                timed_out = true;
                (DiagnosticResult::timed_out_fail("Speed"), None)
            }
        }
    };

    // Enrich adapter details with driver info in tech mode only (WMI query),
    // and run the deep diagnostics with the remaining budget.
    let mut adapter_details = adapter_details;
    let technician = if config.is_tech_mode() {
        if tokio::time::Instant::now() >= deadline {
            timed_out = true;
            None
        } else {
            let tech = tokio::time::timeout_at(deadline, async {
                adapters::enrich_driver_info(&mut adapter_details).await;
                run_technician_diagnostics(config).await
            })
            .await;
            match tech {
                Ok(t) => Some(t),
                Err(_) => {
                    timed_out = true;
                    None
                }
            }
        }
    } else {
        None
    };

    DiagnosticResults {
        timestamp,
        adapters: adapters_result,
        interfaces: interfaces_result,
        gateway: gateway_result,
        dns: dns_result,
        public_ip: public_ip_result,
        latency: latency_result,
        speed: speed_result,
        ports: ports_result,
        interface_details: Some(interface_details),
        adapter_details: Some(adapter_details),
        gateway_details,
        dns_details,
        public_ip_details,
        latency_details: Some(latency_details),
        speed_details,
        port_details: Some(port_details),
        technician,
        timed_out,
    }
}

async fn run_technician_diagnostics(config: &Config) -> TechnicianResults {
    if config.verbose {
        eprintln!("[verbose] Running technician deep diagnostics...");
    }

    // Pre-fetch shared data to avoid duplicate subprocess calls
    let cache = shared_cache::SharedCache::build_for_tech_mode().await;

    let (
        arp_table,
        routing,
        conns,
        listeners,
        dhcp_info,
        proto_stats,
        hw_stats,
        proxy_cfg,
        vpn_adapters,
        fw_info,
        dns_c,
        ipv6_i,
        mtu_i,
        conn_states,
        rdns,
        tls_insp,
        traffic,
    ) = tokio::join!(
        arp::collect(),
        routing_table::collect(),
        connections::collect_with_cache(&cache),
        listening_ports::collect_with_cache(&cache),
        dhcp::collect_with_cache(&cache),
        protocol_stats::collect(),
        adapter_hw_stats::collect_with_cache(&cache),
        proxy::collect(),
        vpn::collect_with_cache(&cache),
        firewall::collect(),
        dns_cache::collect(),
        ipv6::collect_with_cache(&cache),
        mtu::collect(),
        connection_states::collect_with_cache(&cache),
        reverse_dns::collect_with_cache(&cache),
        tls_inspection::collect(),
        traffic_counters::collect_with_cache(&cache),
    );

    // Bufferbloat needs speed test data, run separately
    let bufferbloat = bufferbloat::collect().await;

    TechnicianResults {
        arp_table,
        routing_table: routing,
        active_connections: conns,
        listening_ports: listeners,
        dhcp_info,
        protocol_stats: proto_stats,
        adapter_hw_stats: hw_stats,
        proxy_config: proxy_cfg,
        vpn_info: vpn_adapters,
        firewall_info: fw_info,
        dns_cache: dns_c,
        ipv6_info: ipv6_i,
        mtu_info: mtu_i,
        connection_states: conn_states,
        bufferbloat,
        reverse_dns: rdns,
        tls_inspection: tls_insp,
        traffic_counters: traffic,
    }
}
