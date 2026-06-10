//! Route path analysis (traceroute) — deep diagnostic.
//!
//! One bounded traceroute to a well-known anycast target, parsed into a hop
//! list with technician-grade analysis: first-hop latency (router health),
//! the private→public boundary (where the ISP starts), and the largest
//! latency jump with its segment (LAN / ISP / backbone).

use serde::Serialize;

use super::util;

const TARGET: &str = "1.1.1.1";
const MAX_HOPS: u32 = 15;

#[derive(Debug, Clone, Serialize)]
pub struct Hop {
    pub number: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ip: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub avg_ms: Option<f64>,
    pub timed_out: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct LatencyJump {
    pub from_hop: u32,
    pub to_hop: u32,
    pub delta_ms: f64,
    /// "LAN", "ISP", or "backbone" — where in the path the jump sits.
    pub segment: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct RoutePath {
    pub target: String,
    pub hops: Vec<Hop>,
    pub reached: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub first_hop_ms: Option<f64>,
    /// First hop whose address is public (the ISP boundary).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub isp_boundary_hop: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub largest_jump: Option<LatencyJump>,
    pub assessment: String,
    /// "ok" | "warn" | "fail" (deep modules don't affect exit codes; this is
    /// for rendering/JSON consumers).
    pub level: String,
}

pub async fn collect() -> Option<RoutePath> {
    let transcript = run_traceroute().await?;
    let hops = parse_transcript(&transcript);
    if hops.is_empty() {
        return None;
    }
    Some(analyze_hops(hops))
}

/// Run the platform traceroute, bounded by [`util::TRACE`].
async fn run_traceroute() -> Option<String> {
    #[cfg(windows)]
    {
        let mut cmd = tokio::process::Command::new("tracert");
        cmd.args(["-d", "-h", &MAX_HOPS.to_string(), "-w", "800", TARGET]);
        let output = util::run_with_timeout(cmd, util::TRACE).await?;
        Some(String::from_utf8_lossy(&output.stdout).into_owned())
    }

    #[cfg(unix)]
    {
        let mut cmd = tokio::process::Command::new("traceroute");
        cmd.args([
            "-n",
            "-m",
            &MAX_HOPS.to_string(),
            "-q",
            "2",
            "-w",
            "1",
            TARGET,
        ]);
        if let Some(output) = util::run_with_timeout(cmd, util::TRACE).await {
            if output.status.success() || !output.stdout.is_empty() {
                return Some(String::from_utf8_lossy(&output.stdout).into_owned());
            }
        }
        // Fallback: tracepath (iputils — nearly always present, unprivileged).
        let mut cmd = tokio::process::Command::new("tracepath");
        cmd.args(["-n", "-m", &MAX_HOPS.to_string(), TARGET]);
        let output = util::run_with_timeout(cmd, util::TRACE).await?;
        Some(String::from_utf8_lossy(&output.stdout).into_owned())
    }
}

/// Parse a tracert/traceroute/tracepath transcript into hops. The three
/// formats share enough shape for one tolerant parser:
/// - a leading hop number,
/// - one or more `N ms` / `N.NN ms` / `<1 ms` time tokens,
/// - an IPv4/IPv6 address token (tracert puts it last, traceroute first),
/// - `*` for timed-out probes.
fn parse_transcript(text: &str) -> Vec<Hop> {
    let mut hops: Vec<Hop> = Vec::new();

    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        // Hop lines start with a number.
        let mut parts = trimmed.split_whitespace();
        let Some(first) = parts.next() else { continue };
        let Ok(number) = first.parse::<u32>() else {
            continue;
        };
        if number == 0 || number > MAX_HOPS {
            continue;
        }
        // tracepath prints "1:" / duplicate hop lines; keep the first
        // occurrence per hop number.
        if hops.iter().any(|h: &Hop| h.number == number) {
            continue;
        }

        let rest: Vec<&str> = trimmed.split_whitespace().skip(1).collect();
        let mut times: Vec<f64> = Vec::new();
        let mut ip: Option<String> = None;
        let mut idx = 0;
        while idx < rest.len() {
            let token = rest[idx].trim_end_matches(':').trim_end_matches(',');
            // "<1" then "ms" (tracert sub-millisecond)
            if let Some(stripped) = token.strip_prefix('<') {
                if let Ok(v) = stripped.parse::<f64>() {
                    times.push(v);
                }
            } else if let Ok(v) = token.parse::<f64>() {
                // A bare number followed by "ms" is a time; otherwise it's
                // noise (e.g. tracepath's "pmtu 1500").
                if rest.get(idx + 1).is_some_and(|n| n.starts_with("ms")) {
                    times.push(v);
                    idx += 1; // skip the "ms"
                }
            } else if let Some(stripped) = token.strip_suffix("ms") {
                // traceroute sometimes prints "12.3ms" fused.
                if let Ok(v) = stripped.parse::<f64>() {
                    times.push(v);
                }
            } else if ip.is_none() && looks_like_ip(token) {
                ip = Some(token.to_string());
            }
            idx += 1;
        }

        let timed_out = times.is_empty() && ip.is_none();
        let avg_ms = if times.is_empty() {
            None
        } else {
            Some(times.iter().sum::<f64>() / times.len() as f64)
        };

        hops.push(Hop {
            number,
            ip,
            avg_ms,
            timed_out,
        });
    }

    hops
}

fn looks_like_ip(token: &str) -> bool {
    token.parse::<std::net::IpAddr>().is_ok()
}

fn is_private_ip(ip: &str) -> bool {
    match ip.parse::<std::net::IpAddr>() {
        Ok(std::net::IpAddr::V4(v4)) => v4.is_private() || v4.is_link_local() || is_cgnat_v4(&v4),
        Ok(std::net::IpAddr::V6(v6)) => {
            // fc00::/7 unique-local, fe80::/10 link-local
            let seg = v6.segments()[0];
            (seg & 0xfe00) == 0xfc00 || (seg & 0xffc0) == 0xfe80
        }
        Err(_) => false,
    }
}

fn is_cgnat_v4(v4: &std::net::Ipv4Addr) -> bool {
    // 100.64.0.0/10
    let o = v4.octets();
    o[0] == 100 && (64..128).contains(&o[1])
}

/// Pure analysis over the hop list — unit-testable without a network.
fn analyze_hops(hops: Vec<Hop>) -> RoutePath {
    let reached = hops
        .iter()
        .any(|h| h.ip.as_deref() == Some(TARGET) && !h.timed_out);

    let first_hop_ms = hops.iter().find(|h| h.number == 1).and_then(|h| h.avg_ms);

    // First public-address hop = where the ISP path starts.
    let isp_boundary_hop = hops
        .iter()
        .filter(|h| h.ip.is_some())
        .find(|h| !is_private_ip(h.ip.as_deref().unwrap_or("")))
        .map(|h| h.number);

    // Largest latency increase between consecutive responding hops.
    let responding: Vec<(&Hop, f64)> = hops
        .iter()
        .filter_map(|h| h.avg_ms.map(|ms| (h, ms)))
        .collect();
    let largest_jump = responding
        .windows(2)
        .filter_map(|w| {
            let (from, from_ms) = w[0];
            let (to, to_ms) = w[1];
            let delta = to_ms - from_ms;
            if delta <= 0.0 {
                return None;
            }
            let segment = match isp_boundary_hop {
                Some(boundary) if to.number < boundary => "LAN",
                Some(boundary) if to.number <= boundary + 3 => "ISP",
                Some(_) => "backbone",
                None => "LAN",
            };
            Some(LatencyJump {
                from_hop: from.number,
                to_hop: to.number,
                delta_ms: delta,
                segment: segment.to_string(),
            })
        })
        .max_by(|a, b| {
            a.delta_ms
                .partial_cmp(&b.delta_ms)
                .unwrap_or(std::cmp::Ordering::Equal)
        });

    // Assessment.
    let (assessment, level) = if !reached {
        (
            format!("Target {} not reached within {} hops", TARGET, MAX_HOPS),
            "fail",
        )
    } else if first_hop_ms.is_some_and(|ms| ms > 10.0) {
        (
            format!(
                "First hop (your router) is slow at {:.0}ms — check Wi-Fi signal or router load",
                first_hop_ms.unwrap_or(0.0)
            ),
            "warn",
        )
    } else if largest_jump
        .as_ref()
        .is_some_and(|j| j.delta_ms > 100.0 && j.segment == "ISP")
    {
        (
            "Large latency jump inside the ISP segment — possible congestion".to_string(),
            "warn",
        )
    } else {
        ("Path looks healthy".to_string(), "ok")
    };

    RoutePath {
        target: TARGET.to_string(),
        hops,
        reached,
        first_hop_ms,
        isp_boundary_hop,
        largest_jump,
        assessment,
        level: level.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TRACERT_TRANSCRIPT: &str = "\
Tracing route to 1.1.1.1 over a maximum of 15 hops

  1    <1 ms    <1 ms    <1 ms  192.168.1.1
  2     8 ms     9 ms     8 ms  100.64.0.1
  3    12 ms    11 ms    13 ms  68.86.92.25
  4     *        *        *     Request timed out.
  5    13 ms    14 ms    12 ms  1.1.1.1

Trace complete.";

    const TRACEROUTE_TRANSCRIPT: &str = "\
traceroute to 1.1.1.1 (1.1.1.1), 15 hops max, 60 byte packets
 1  192.168.1.1  0.512 ms  0.498 ms
 2  100.64.0.1  8.123 ms  8.077 ms
 3  68.86.92.25  12.401 ms  12.388 ms
 4  * *
 5  1.1.1.1  13.102 ms  13.097 ms";

    #[test]
    fn parses_tracert_transcript() {
        let hops = parse_transcript(TRACERT_TRANSCRIPT);
        assert_eq!(hops.len(), 5);
        assert_eq!(hops[0].ip.as_deref(), Some("192.168.1.1"));
        assert_eq!(hops[0].avg_ms, Some(1.0)); // "<1 ms" × 3
        assert!(hops[3].timed_out);
        assert_eq!(hops[4].ip.as_deref(), Some("1.1.1.1"));
    }

    #[test]
    fn parses_traceroute_transcript() {
        let hops = parse_transcript(TRACEROUTE_TRANSCRIPT);
        assert_eq!(hops.len(), 5);
        assert_eq!(hops[1].ip.as_deref(), Some("100.64.0.1"));
        assert!(hops[1].avg_ms.is_some_and(|ms| (ms - 8.1).abs() < 0.1));
        assert!(hops[3].timed_out);
    }

    #[test]
    fn analysis_finds_boundary_and_reachability() {
        let path = analyze_hops(parse_transcript(TRACERT_TRANSCRIPT));
        assert!(path.reached);
        // 192.168.1.1 private, 100.64.0.1 CGNAT (treated private) — first
        // public hop is hop 3.
        assert_eq!(path.isp_boundary_hop, Some(3));
        assert_eq!(path.level, "ok");
        assert!(path.first_hop_ms.is_some());
    }

    #[test]
    fn unreached_target_is_fail_level() {
        let hops = vec![
            Hop {
                number: 1,
                ip: Some("192.168.1.1".to_string()),
                avg_ms: Some(1.0),
                timed_out: false,
            },
            Hop {
                number: 2,
                ip: None,
                avg_ms: None,
                timed_out: true,
            },
        ];
        let path = analyze_hops(hops);
        assert!(!path.reached);
        assert_eq!(path.level, "fail");
    }

    #[test]
    fn slow_first_hop_warns() {
        let hops = vec![
            Hop {
                number: 1,
                ip: Some("192.168.1.1".to_string()),
                avg_ms: Some(35.0),
                timed_out: false,
            },
            Hop {
                number: 2,
                ip: Some("1.1.1.1".to_string()),
                avg_ms: Some(40.0),
                timed_out: false,
            },
        ];
        let path = analyze_hops(hops);
        assert_eq!(path.level, "warn");
        assert!(path.assessment.contains("router"));
    }

    #[test]
    fn cgnat_addresses_count_as_private() {
        assert!(is_private_ip("100.64.0.1"));
        assert!(is_private_ip("192.168.1.1"));
        assert!(!is_private_ip("68.86.92.25"));
        assert!(!is_private_ip("1.1.1.1"));
    }
}
