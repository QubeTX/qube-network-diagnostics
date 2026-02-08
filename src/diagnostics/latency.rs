use serde::Serialize;

use super::DiagnosticResult;

#[derive(Debug, Clone, Serialize)]
pub struct LatencyResult {
    pub host: String,
    pub label: String,
    pub reachable: bool,
    pub min_ms: Option<f64>,
    pub avg_ms: Option<f64>,
    pub max_ms: Option<f64>,
    pub jitter_ms: Option<f64>,
    pub packet_loss: f64,
}

const TARGETS: &[(&str, &str)] = &[
    ("1.1.1.1", "Cloudflare"),
    ("8.8.8.8", "Google DNS"),
    ("208.67.222.222", "OpenDNS"),
];

pub async fn check() -> (DiagnosticResult, Vec<LatencyResult>) {
    let mut results = Vec::new();

    for (host, label) in TARGETS {
        let result = ping_multiple(host, label, 4).await;
        results.push(result);
    }

    let reachable = results.iter().filter(|r| r.reachable).count();
    let total = results.len();

    let result = if reachable == 0 {
        DiagnosticResult::fail("Latency", "All endpoints unreachable")
    } else {
        // Check average latencies
        let avg_latency: f64 = results
            .iter()
            .filter_map(|r| r.avg_ms)
            .sum::<f64>()
            / reachable as f64;

        if avg_latency > 200.0 {
            DiagnosticResult::warn("Latency", format!("High latency (~{:.0}ms avg)", avg_latency))
        } else if avg_latency > 100.0 {
            DiagnosticResult::warn("Latency", format!("Moderate latency (~{:.0}ms avg)", avg_latency))
        } else if reachable < total {
            DiagnosticResult::warn(
                "Latency",
                format!("{}/{} endpoints reachable", reachable, total),
            )
        } else {
            DiagnosticResult::ok("Latency", "Low latency to all endpoints")
        }
    };

    (result, results)
}

async fn ping_multiple(host: &str, label: &str, count: u32) -> LatencyResult {
    #[cfg(windows)]
    let output = tokio::process::Command::new("ping")
        .args(["-n", &count.to_string(), "-w", "2000", host])
        .output()
        .await;

    #[cfg(unix)]
    let output = tokio::process::Command::new("ping")
        .args(["-c", &count.to_string(), "-W", "2", host])
        .output()
        .await;

    match output {
        Ok(out) if out.status.success() => {
            let text = String::from_utf8_lossy(&out.stdout);
            parse_ping_output(&text, host, label)
        }
        _ => LatencyResult {
            host: host.to_string(),
            label: label.to_string(),
            reachable: false,
            min_ms: None,
            avg_ms: None,
            max_ms: None,
            jitter_ms: None,
            packet_loss: 100.0,
        },
    }
}

fn parse_ping_output(text: &str, host: &str, label: &str) -> LatencyResult {
    let mut times: Vec<f64> = Vec::new();
    let mut packet_loss = 0.0;

    for line in text.lines() {
        // Parse individual reply times
        if let Some(time) = extract_time(line) {
            times.push(time);
        }
        // Parse packet loss percentage
        if line.contains("loss") || line.contains("Lost") {
            if let Some(pct) = extract_loss_pct(line) {
                packet_loss = pct;
            }
        }
    }

    if times.is_empty() {
        return LatencyResult {
            host: host.to_string(),
            label: label.to_string(),
            reachable: false,
            min_ms: None,
            avg_ms: None,
            max_ms: None,
            jitter_ms: None,
            packet_loss: 100.0,
        };
    }

    let min = times.iter().cloned().fold(f64::INFINITY, f64::min);
    let max = times.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
    let avg = times.iter().sum::<f64>() / times.len() as f64;

    // Jitter = average of absolute differences between consecutive pings
    let jitter = if times.len() > 1 {
        let diffs: Vec<f64> = times.windows(2).map(|w| (w[1] - w[0]).abs()).collect();
        Some(diffs.iter().sum::<f64>() / diffs.len() as f64)
    } else {
        None
    };

    LatencyResult {
        host: host.to_string(),
        label: label.to_string(),
        reachable: true,
        min_ms: Some(min),
        avg_ms: Some(avg),
        max_ms: Some(max),
        jitter_ms: jitter,
        packet_loss,
    }
}

fn extract_time(line: &str) -> Option<f64> {
    // "time=1.23ms" or "time=1ms" or "time<1ms"
    if let Some(pos) = line.find("time=") {
        let after = &line[pos + 5..];
        let num: String = after
            .chars()
            .take_while(|c| c.is_ascii_digit() || *c == '.')
            .collect();
        return num.parse().ok();
    }
    if let Some(pos) = line.find("time<") {
        let after = &line[pos + 5..];
        let num: String = after
            .chars()
            .take_while(|c| c.is_ascii_digit() || *c == '.')
            .collect();
        return num.parse().ok();
    }
    None
}

fn extract_loss_pct(line: &str) -> Option<f64> {
    // "0% loss" or "(0% loss)" or "0% packet loss"
    if let Some(pos) = line.find('%') {
        // Walk backwards to find the number
        let before = &line[..pos];
        let num_str: String = before
            .chars()
            .rev()
            .take_while(|c| c.is_ascii_digit() || *c == '.')
            .collect::<String>()
            .chars()
            .rev()
            .collect();
        return num_str.parse().ok();
    }
    None
}
