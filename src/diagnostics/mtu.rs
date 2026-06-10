use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct MtuInfo {
    pub interface: String,
    pub mtu: u32,
}

pub async fn collect() -> Option<Vec<MtuInfo>> {
    let mut results = Vec::new();

    #[cfg(windows)]
    {
        let mut cmd = tokio::process::Command::new("netsh");
        cmd.args(["interface", "ipv4", "show", "subinterfaces"]);
        if let Some(output) = super::util::run_with_timeout(cmd, super::util::QUICK).await {
            let text = String::from_utf8_lossy(&output.stdout);
            for line in text.lines() {
                let parts: Vec<&str> = line.split_whitespace().collect();
                // Format: MTU  MediaSenseState   Bytes In  Bytes Out  Interface
                if parts.len() >= 5 {
                    if let Ok(mtu) = parts[0].parse::<u32>() {
                        let iface = parts[4..].join(" ");
                        results.push(MtuInfo {
                            interface: iface,
                            mtu,
                        });
                    }
                }
            }
        }
    }

    #[cfg(target_os = "linux")]
    {
        let mut cmd = tokio::process::Command::new("ip");
        cmd.args(["link", "show"]);
        if let Some(output) = super::util::run_with_timeout(cmd, super::util::QUICK).await {
            let text = String::from_utf8_lossy(&output.stdout);
            for line in text.lines() {
                if line.contains("mtu") {
                    let parts: Vec<&str> = line.split_whitespace().collect();
                    if let Some(name_idx) = parts
                        .iter()
                        .position(|p| p.ends_with(':') && !p.starts_with(' '))
                    {
                        let name = parts[name_idx].trim_end_matches(':');
                        if let Some(mtu_idx) = parts.iter().position(|p| *p == "mtu") {
                            if let Some(mtu_val) = parts.get(mtu_idx + 1) {
                                if let Ok(mtu) = mtu_val.parse::<u32>() {
                                    results.push(MtuInfo {
                                        interface: name.to_string(),
                                        mtu,
                                    });
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    #[cfg(target_os = "macos")]
    {
        let cmd = tokio::process::Command::new("ifconfig");
        if let Some(output) = super::util::run_with_timeout(cmd, super::util::QUICK).await {
            let text = String::from_utf8_lossy(&output.stdout);
            let mut current_iface = String::new();

            for line in text.lines() {
                if !line.starts_with('\t') && !line.starts_with(' ') {
                    current_iface = line.split(':').next().unwrap_or("").to_string();
                }
                if line.contains("mtu") {
                    let parts: Vec<&str> = line.split_whitespace().collect();
                    if let Some(mtu_idx) = parts.iter().position(|p| *p == "mtu") {
                        if let Some(mtu_val) = parts.get(mtu_idx + 1) {
                            if let Ok(mtu) = mtu_val.parse::<u32>() {
                                results.push(MtuInfo {
                                    interface: current_iface.clone(),
                                    mtu,
                                });
                            }
                        }
                    }
                }
            }
        }
    }

    if results.is_empty() {
        None
    } else {
        Some(results)
    }
}

// ── Path MTU discovery (v3.4.0+) ────────────────────────────────────────────
//
// The listing above only reads the INTERFACE MTU. This probe measures the
// PATH MTU with a don't-fragment binary search against 1.1.1.1 — a clamped
// path (VPN/tunnel overhead) or broken PMTUD causes mysterious stalls on
// large transfers that interface MTUs can't explain.

const PROBE_TARGET: &str = "1.1.1.1";
/// ICMP payload search bounds; payload + 28 (IP+ICMP headers) = MTU, so this
/// covers MTUs 1228–1500.
const PAYLOAD_LO: u32 = 1200;
const PAYLOAD_HI: u32 = 1472;
const MAX_PROBES: u32 = 8;

#[derive(Debug, Clone, Serialize)]
pub struct PathMtu {
    pub target: String,
    pub path_mtu: u32,
    /// True when the path MTU is below the standard Ethernet 1500.
    pub clamped: bool,
    pub probes: u32,
    pub assessment: String,
    pub level: String,
}

pub async fn probe_path_mtu() -> Option<PathMtu> {
    // Fast path: a full 1500-byte frame passing with DF means no clamp.
    let mut probes = 1;
    if probe_df(PAYLOAD_HI).await? {
        let (assessment, level) = classify_path_mtu(payload_to_mtu(PAYLOAD_HI));
        return Some(PathMtu {
            target: PROBE_TARGET.to_string(),
            path_mtu: payload_to_mtu(PAYLOAD_HI),
            clamped: false,
            probes,
            assessment,
            level,
        });
    }

    // The floor failing too usually means DF probes are filtered outright
    // (or PMTUD is broken) — report that rather than a fake number.
    probes += 1;
    if !probe_df(PAYLOAD_LO).await? {
        return Some(PathMtu {
            target: PROBE_TARGET.to_string(),
            path_mtu: 0,
            clamped: true,
            probes,
            assessment: format!(
                "Even {}-byte don't-fragment probes fail — path MTU discovery appears broken (ICMP filtered?)",
                payload_to_mtu(PAYLOAD_LO)
            ),
            level: "fail".to_string(),
        });
    }

    // Binary search the largest passing payload in (lo_pass, hi_fail).
    let mut lo = PAYLOAD_LO; // known pass
    let mut hi = PAYLOAD_HI; // known fail
    while probes < MAX_PROBES {
        let Some(mid) = next_probe(lo, hi) else { break };
        probes += 1;
        if probe_df(mid).await? {
            lo = mid;
        } else {
            hi = mid;
        }
    }

    let path_mtu = payload_to_mtu(lo);
    let (assessment, level) = classify_path_mtu(path_mtu);
    Some(PathMtu {
        target: PROBE_TARGET.to_string(),
        path_mtu,
        clamped: path_mtu < 1500,
        probes,
        assessment,
        level,
    })
}

/// Midpoint for the binary search; `None` when the bounds have converged.
fn next_probe(lo_pass: u32, hi_fail: u32) -> Option<u32> {
    if hi_fail <= lo_pass + 1 {
        return None;
    }
    Some(lo_pass + (hi_fail - lo_pass) / 2)
}

/// ICMP payload bytes → MTU (20 IP + 8 ICMP header bytes).
fn payload_to_mtu(payload: u32) -> u32 {
    payload + 28
}

fn classify_path_mtu(mtu: u32) -> (String, String) {
    if mtu >= 1500 {
        (
            "Full 1500-byte path — no clamping".to_string(),
            "ok".to_string(),
        )
    } else if mtu >= 1492 {
        (
            "1492-byte path — standard PPPoE overhead, normal for DSL".to_string(),
            "ok".to_string(),
        )
    } else if mtu >= 1400 {
        (
            format!(
                "Path clamped to {} — typical of a tunnel or VPN in the path",
                mtu
            ),
            "ok".to_string(),
        )
    } else {
        (
            format!(
                "Path clamped to {} — heavy tunnel overhead; large packets risk fragmentation stalls",
                mtu
            ),
            "warn".to_string(),
        )
    }
}

/// One don't-fragment ping with the given payload size. `Some(true)` = reply
/// received; `Some(false)` = DF rejection/no reply; `None` = ping itself
/// unavailable (probe aborts and the section is omitted).
async fn probe_df(payload: u32) -> Option<bool> {
    #[cfg(windows)]
    let args: Vec<String> = vec![
        "-f".into(),
        "-l".into(),
        payload.to_string(),
        "-n".into(),
        "1".into(),
        "-w".into(),
        "1500".into(),
        PROBE_TARGET.into(),
    ];
    #[cfg(target_os = "linux")]
    let args: Vec<String> = vec![
        "-M".into(),
        "do".into(),
        "-s".into(),
        payload.to_string(),
        "-c".into(),
        "1".into(),
        "-W".into(),
        "2".into(),
        PROBE_TARGET.into(),
    ];
    #[cfg(target_os = "macos")]
    let args: Vec<String> = vec![
        "-D".into(),
        "-s".into(),
        payload.to_string(),
        "-c".into(),
        "1".into(),
        PROBE_TARGET.into(),
    ];

    let mut cmd = tokio::process::Command::new("ping");
    cmd.args(&args);
    let output = super::util::run_with_timeout(cmd, super::util::SLOW).await?;
    Some(output.status.success())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn payload_mtu_arithmetic() {
        assert_eq!(payload_to_mtu(1472), 1500);
        assert_eq!(payload_to_mtu(1464), 1492);
        assert_eq!(payload_to_mtu(1200), 1228);
    }

    #[test]
    fn binary_search_converges() {
        // Simulate a 1400-byte path MTU (payload 1372): probe passes ≤1372.
        let true_payload_limit = 1372;
        let mut lo = PAYLOAD_LO;
        let mut hi = PAYLOAD_HI;
        let mut steps = 0;
        while let Some(mid) = next_probe(lo, hi) {
            steps += 1;
            if mid <= true_payload_limit {
                lo = mid;
            } else {
                hi = mid;
            }
            assert!(steps < 12, "search must converge");
        }
        assert_eq!(payload_to_mtu(lo), 1400);
        assert!(steps <= 9);
    }

    #[test]
    fn classify_thresholds() {
        assert_eq!(classify_path_mtu(1500).1, "ok");
        assert_eq!(classify_path_mtu(1492).1, "ok");
        assert_eq!(classify_path_mtu(1420).1, "ok");
        assert_eq!(classify_path_mtu(1340).1, "warn");
    }
}
