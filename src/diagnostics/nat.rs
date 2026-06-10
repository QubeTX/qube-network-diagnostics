//! NAT topology analysis — deep diagnostic.
//!
//! Pure post-processing over data other modules already collected (gateway
//! IP, public IP, traceroute hops): detects double NAT (two private-range
//! hops at the head of the path in different subnets) and carrier-grade NAT
//! (any hop or the public IP inside 100.64.0.0/10 — port forwarding and
//! hosting are impossible behind CGNAT, high technician value). Sync by
//! design: it derives from other modules' outputs after they complete.

use serde::Serialize;

use super::public_ip::PublicIpInfo;
use super::route_path::Hop;

#[derive(Debug, Clone, Serialize)]
pub struct NatAnalysis {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gateway_ip: Option<String>,
    pub gateway_is_private: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hop2_ip: Option<String>,
    pub hop2_is_private: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub public_ip: Option<String>,
    pub cgnat_detected: bool,
    pub double_nat_suspected: bool,
    pub nat_layers_estimate: u8,
    pub assessment: String,
    pub level: String,
}

/// Analyze NAT topology. Returns `None` (section omitted) when there is
/// neither a gateway nor any traceroute data to reason from.
pub fn analyze(
    gateway_ip: Option<&str>,
    public_ip: Option<&PublicIpInfo>,
    hops: Option<&[Hop]>,
) -> Option<NatAnalysis> {
    if gateway_ip.is_none() && hops.is_none_or(|h| h.is_empty()) {
        return None;
    }

    let gateway_is_private = gateway_ip.is_some_and(is_rfc1918_or_cgnat);

    // Hop 2 = the device just past the local router. Private there (in a
    // different subnet from the gateway) usually means a second NAT layer —
    // an ISP modem/router in front of the user's own router.
    let hop2 = hops.and_then(|h| {
        h.iter()
            .find(|hop| hop.number == 2)
            .and_then(|hop| hop.ip.clone())
    });
    let hop2_is_private = hop2.as_deref().is_some_and(is_rfc1918_or_cgnat);

    let public_addr = public_ip.map(|p| p.ip.clone());

    // CGNAT: the public-facing address (or any path hop) is in 100.64/10.
    let cgnat_detected = public_addr.as_deref().is_some_and(is_cgnat)
        || hops.is_some_and(|h| h.iter().filter_map(|hop| hop.ip.as_deref()).any(is_cgnat));

    let double_nat_suspected = gateway_is_private
        && hop2_is_private
        && !same_slash24(gateway_ip.unwrap_or(""), hop2.as_deref().unwrap_or(""));

    let nat_layers_estimate =
        u8::from(gateway_is_private) + u8::from(double_nat_suspected) + u8::from(cgnat_detected);

    let (assessment, level) = if cgnat_detected {
        (
            "Carrier-grade NAT detected — inbound connections, port forwarding, and hosting won't work; contact the ISP for a public IP if needed",
            "warn",
        )
    } else if double_nat_suspected {
        (
            "Double NAT suspected — two private networks stacked; consider bridging the ISP modem",
            "warn",
        )
    } else if gateway_is_private {
        ("Single NAT — the normal home/office setup", "ok")
    } else {
        (
            "Gateway has a public address — no NAT (rare; verify the firewall is intentional)",
            "warn",
        )
    };

    Some(NatAnalysis {
        gateway_ip: gateway_ip.map(|s| s.to_string()),
        gateway_is_private,
        hop2_ip: hop2,
        hop2_is_private,
        public_ip: public_addr,
        cgnat_detected,
        double_nat_suspected,
        nat_layers_estimate,
        assessment: assessment.to_string(),
        level: level.to_string(),
    })
}

fn is_rfc1918_or_cgnat(ip: &str) -> bool {
    match ip.parse::<std::net::IpAddr>() {
        Ok(std::net::IpAddr::V4(v4)) => v4.is_private() || v4.is_link_local() || is_cgnat_v4(&v4),
        Ok(std::net::IpAddr::V6(v6)) => {
            let seg = v6.segments()[0];
            (seg & 0xfe00) == 0xfc00 || (seg & 0xffc0) == 0xfe80
        }
        Err(_) => false,
    }
}

fn is_cgnat(ip: &str) -> bool {
    match ip.parse::<std::net::IpAddr>() {
        Ok(std::net::IpAddr::V4(v4)) => is_cgnat_v4(&v4),
        _ => false,
    }
}

fn is_cgnat_v4(v4: &std::net::Ipv4Addr) -> bool {
    let o = v4.octets();
    o[0] == 100 && (64..128).contains(&o[1])
}

/// Same /24 — used to tell "router seen twice" apart from "two NAT layers".
fn same_slash24(a: &str, b: &str) -> bool {
    let parse =
        |s: &str| -> Option<[u8; 4]> { s.parse::<std::net::Ipv4Addr>().ok().map(|v| v.octets()) };
    match (parse(a), parse(b)) {
        (Some(x), Some(y)) => x[0] == y[0] && x[1] == y[1] && x[2] == y[2],
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hop(number: u32, ip: &str) -> Hop {
        Hop {
            number,
            ip: Some(ip.to_string()),
            avg_ms: Some(1.0),
            timed_out: false,
        }
    }

    fn public(ip: &str) -> PublicIpInfo {
        PublicIpInfo {
            ip: ip.to_string(),
            lookup_time_ms: 10.0,
            behind_nat: true,
            city: None,
            region: None,
            country: None,
            isp: None,
            org: None,
        }
    }

    #[test]
    fn single_nat_is_ok() {
        let hops = [hop(1, "192.168.1.1"), hop(2, "68.86.92.25")];
        let n = analyze(
            Some("192.168.1.1"),
            Some(&public("203.0.113.5")),
            Some(&hops),
        )
        .unwrap();
        assert!(!n.double_nat_suspected);
        assert!(!n.cgnat_detected);
        assert_eq!(n.nat_layers_estimate, 1);
        assert_eq!(n.level, "ok");
    }

    #[test]
    fn double_nat_detected_across_subnets() {
        let hops = [hop(1, "192.168.1.1"), hop(2, "10.0.0.1")];
        let n = analyze(
            Some("192.168.1.1"),
            Some(&public("203.0.113.5")),
            Some(&hops),
        )
        .unwrap();
        assert!(n.double_nat_suspected);
        assert_eq!(n.level, "warn");
        assert_eq!(n.nat_layers_estimate, 2);
    }

    #[test]
    fn same_subnet_hop2_is_not_double_nat() {
        let hops = [hop(1, "192.168.1.1"), hop(2, "192.168.1.254")];
        let n = analyze(Some("192.168.1.1"), None, Some(&hops)).unwrap();
        assert!(!n.double_nat_suspected);
    }

    #[test]
    fn cgnat_detected_from_public_ip() {
        let n = analyze(Some("192.168.1.1"), Some(&public("100.72.13.4")), None).unwrap();
        assert!(n.cgnat_detected);
        assert_eq!(n.level, "warn");
        assert!(n.assessment.contains("Carrier-grade"));
    }

    #[test]
    fn cgnat_detected_from_path_hop() {
        let hops = [hop(1, "192.168.1.1"), hop(2, "100.64.0.1")];
        let n = analyze(
            Some("192.168.1.1"),
            Some(&public("203.0.113.5")),
            Some(&hops),
        )
        .unwrap();
        assert!(n.cgnat_detected);
    }

    #[test]
    fn no_inputs_returns_none() {
        assert!(analyze(None, None, None).is_none());
        assert!(analyze(None, None, Some(&[])).is_none());
    }

    #[test]
    fn public_gateway_flagged() {
        let n = analyze(Some("203.0.113.1"), None, Some(&[hop(1, "203.0.113.1")])).unwrap();
        assert!(!n.gateway_is_private);
        assert_eq!(n.level, "warn");
    }
}
