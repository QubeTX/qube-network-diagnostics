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
}

impl DiagnosticResult {
    pub fn ok(category: impl Into<String>, summary: impl Into<String>) -> Self {
        Self {
            category: category.into(),
            status: DiagnosticStatus::Ok,
            summary: summary.into(),
            details: None,
        }
    }

    pub fn warn(category: impl Into<String>, summary: impl Into<String>) -> Self {
        Self {
            category: category.into(),
            status: DiagnosticStatus::Warn,
            summary: summary.into(),
            details: None,
        }
    }

    pub fn fail(category: impl Into<String>, summary: impl Into<String>) -> Self {
        Self {
            category: category.into(),
            status: DiagnosticStatus::Fail,
            summary: summary.into(),
            details: None,
        }
    }

    pub fn skip(category: impl Into<String>, summary: impl Into<String>) -> Self {
        Self {
            category: category.into(),
            status: DiagnosticStatus::Skip,
            summary: summary.into(),
            details: None,
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

pub async fn run_all(config: &Config) -> DiagnosticResults {
    let timestamp = chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string();

    // Run core diagnostics concurrently
    let (
        (adapters_result, adapter_details),
        (interfaces_result, interface_details),
        (gateway_result, gateway_details),
        (dns_result, dns_details),
        (public_ip_result, public_ip_details),
        (latency_result, latency_details),
        (ports_result, port_details),
    ) = tokio::join!(
        adapters::check(),
        interfaces::check(),
        gateway::check(),
        dns::check(),
        public_ip::check(),
        latency::check(),
        ports::check(),
    );

    // Run speed test sequentially (it needs to saturate bandwidth)
    let (speed_result, speed_details) = if config.skip_speed {
        (
            DiagnosticResult::skip("Speed", "Speed test skipped (--fast)"),
            None,
        )
    } else {
        speed::check(config).await
    };

    // Enrich adapter details with driver info in tech mode only (WMI query)
    let mut adapter_details = adapter_details;
    let technician = if config.is_tech_mode() {
        adapters::enrich_driver_info(&mut adapter_details).await;
        Some(run_technician_diagnostics(config).await)
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
