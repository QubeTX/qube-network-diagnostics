//! Shared ping invocation and output parsing.
//!
//! The same ping command construction and reply/loss parsing used to live in
//! three places (`gateway`, `latency`, `bufferbloat`) with slight drift. This
//! module is the single home: build args per platform, run with a budget
//! sized to the probe count, and parse replies into [`PingStats`].

use super::util;

/// Parsed result of one ping burst.
#[derive(Debug, Clone)]
pub struct PingStats {
    /// Probes sent (the `-n`/`-c` count requested).
    pub sent: u32,
    /// Round-trip times of the replies that came back, in milliseconds.
    pub times_ms: Vec<f64>,
    /// Packet loss percentage — parsed from the summary line when present,
    /// otherwise computed from `sent` vs replies.
    pub packet_loss_pct: f64,
}

impl PingStats {
    /// An all-lost burst (used when the subprocess fails or times out).
    pub fn all_lost(sent: u32) -> Self {
        Self {
            sent,
            times_ms: Vec::new(),
            packet_loss_pct: 100.0,
        }
    }

    pub fn received(&self) -> u32 {
        self.times_ms.len() as u32
    }

    pub fn avg_ms(&self) -> Option<f64> {
        if self.times_ms.is_empty() {
            None
        } else {
            Some(self.times_ms.iter().sum::<f64>() / self.times_ms.len() as f64)
        }
    }

    pub fn min_ms(&self) -> Option<f64> {
        self.times_ms.iter().cloned().reduce(f64::min)
    }

    pub fn max_ms(&self) -> Option<f64> {
        self.times_ms.iter().cloned().reduce(f64::max)
    }

    /// Average of absolute differences between consecutive replies — the same
    /// jitter definition the latency module has always reported.
    pub fn jitter_ms(&self) -> Option<f64> {
        if self.times_ms.len() > 1 {
            let diffs: Vec<f64> = self
                .times_ms
                .windows(2)
                .map(|w| (w[1] - w[0]).abs())
                .collect();
            Some(diffs.iter().sum::<f64>() / diffs.len() as f64)
        } else {
            None
        }
    }

    /// Combine two bursts against the same target (e.g. a confirmation burst
    /// after a fully-lost first burst). Loss is recomputed from the combined
    /// counts.
    pub fn merged_with(mut self, other: PingStats) -> PingStats {
        self.sent += other.sent;
        self.times_ms.extend(other.times_ms);
        self.packet_loss_pct = if self.sent == 0 {
            0.0
        } else {
            (self.sent.saturating_sub(self.received())) as f64 / self.sent as f64 * 100.0
        };
        self
    }
}

/// Platform-specific ping arguments for `count` probes with a 2s per-reply
/// timeout. Moved verbatim from the latency module.
pub fn ping_args(host: &str, count: u32) -> Vec<String> {
    #[cfg(windows)]
    {
        vec![
            "-n".to_string(),
            count.to_string(),
            "-w".to_string(),
            "2000".to_string(),
            host.to_string(),
        ]
    }

    #[cfg(target_os = "macos")]
    {
        vec![
            "-c".to_string(),
            count.to_string(),
            "-W".to_string(),
            "2000".to_string(),
            host.to_string(),
        ]
    }

    #[cfg(all(unix, not(target_os = "macos")))]
    {
        vec![
            "-c".to_string(),
            count.to_string(),
            "-W".to_string(),
            "2".to_string(),
            host.to_string(),
        ]
    }
}

/// Run a ping burst of `count` probes against `host`.
///
/// Returns the stdout transcript when the process finished and reported
/// success (at least one reply on every supported platform), `None` on a
/// spawn failure, timeout, or zero replies. The budget scales with the count
/// via [`util::ping_budget`] so long bursts aren't truncated by a fixed cap.
pub async fn run_ping(host: &str, count: u32) -> Option<String> {
    let mut cmd = tokio::process::Command::new("ping");
    cmd.args(ping_args(host, count));
    let output = util::run_with_timeout(cmd, util::ping_budget(count)).await?;
    if output.status.success() {
        Some(String::from_utf8_lossy(&output.stdout).into_owned())
    } else {
        None
    }
}

/// Parse a ping transcript into [`PingStats`].
///
/// Reply times come from `time=`/`time<` markers (Windows/Linux/macOS); the
/// loss percentage comes from the summary line (`25% loss`, `Lost = 1 (25%
/// loss)`, `25% packet loss`), falling back to a count-based computation when
/// no summary parsed.
pub fn parse_ping(stdout: &str, sent: u32) -> PingStats {
    let mut times: Vec<f64> = Vec::new();
    let mut summary_loss: Option<f64> = None;

    for line in stdout.lines() {
        if let Some(time) = extract_time(line) {
            times.push(time);
        }
        if (line.contains("loss") || line.contains("Lost")) && summary_loss.is_none() {
            summary_loss = extract_loss_pct(line);
        }
    }

    let packet_loss_pct = summary_loss.unwrap_or_else(|| {
        if sent == 0 {
            0.0
        } else {
            (sent.saturating_sub(times.len() as u32)) as f64 / sent as f64 * 100.0
        }
    });

    PingStats {
        sent,
        times_ms: times,
        packet_loss_pct,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[cfg(target_os = "macos")]
    fn macos_ping_timeout_uses_milliseconds() {
        assert_eq!(
            ping_args("1.1.1.1", 4),
            vec!["-c", "4", "-W", "2000", "1.1.1.1"]
        );
    }

    #[test]
    #[cfg(all(unix, not(target_os = "macos")))]
    fn linux_ping_timeout_uses_seconds() {
        assert_eq!(
            ping_args("1.1.1.1", 4),
            vec!["-c", "4", "-W", "2", "1.1.1.1"]
        );
    }

    #[test]
    #[cfg(windows)]
    fn windows_ping_timeout_uses_milliseconds() {
        assert_eq!(
            ping_args("1.1.1.1", 4),
            vec!["-n", "4", "-w", "2000", "1.1.1.1"]
        );
    }

    const WINDOWS_TRANSCRIPT: &str = "\
Pinging 1.1.1.1 with 32 bytes of data:
Reply from 1.1.1.1: bytes=32 time=12ms TTL=58
Reply from 1.1.1.1: bytes=32 time=11ms TTL=58
Reply from 1.1.1.1: bytes=32 time=14ms TTL=58
Request timed out.

Ping statistics for 1.1.1.1:
    Packets: Sent = 4, Received = 3, Lost = 1 (25% loss),
Approximate round trip times in milli-seconds:
    Minimum = 11ms, Maximum = 14ms, Average = 12ms";

    const LINUX_TRANSCRIPT: &str = "\
PING 1.1.1.1 (1.1.1.1) 56(84) bytes of data.
64 bytes from 1.1.1.1: icmp_seq=1 ttl=58 time=12.3 ms
64 bytes from 1.1.1.1: icmp_seq=2 ttl=58 time=11.8 ms
64 bytes from 1.1.1.1: icmp_seq=4 ttl=58 time=13.1 ms

--- 1.1.1.1 ping statistics ---
4 packets transmitted, 3 received, 25% packet loss, time 3004ms
rtt min/avg/max/mdev = 11.800/12.400/13.100/0.535 ms";

    const MACOS_TRANSCRIPT: &str = "\
PING 1.1.1.1 (1.1.1.1): 56 data bytes
64 bytes from 1.1.1.1: icmp_seq=0 ttl=58 time=12.345 ms
64 bytes from 1.1.1.1: icmp_seq=1 ttl=58 time=11.872 ms

--- 1.1.1.1 ping statistics ---
2 packets transmitted, 2 packets received, 0.0% packet loss
round-trip min/avg/max/stddev = 11.872/12.108/12.345/0.236 ms";

    #[test]
    fn parses_windows_transcript() {
        let stats = parse_ping(WINDOWS_TRANSCRIPT, 4);
        assert_eq!(stats.received(), 3);
        assert_eq!(stats.packet_loss_pct, 25.0);
        assert_eq!(stats.min_ms(), Some(11.0));
        assert_eq!(stats.max_ms(), Some(14.0));
    }

    #[test]
    fn parses_linux_transcript() {
        let stats = parse_ping(LINUX_TRANSCRIPT, 4);
        assert_eq!(stats.received(), 3);
        assert_eq!(stats.packet_loss_pct, 25.0);
        assert!((stats.avg_ms().unwrap() - 12.4).abs() < 0.01);
    }

    #[test]
    fn parses_macos_transcript() {
        let stats = parse_ping(MACOS_TRANSCRIPT, 2);
        assert_eq!(stats.received(), 2);
        assert_eq!(stats.packet_loss_pct, 0.0);
        assert!(stats.jitter_ms().unwrap() > 0.0);
    }

    #[test]
    fn zero_reply_transcript_is_all_lost() {
        let stats = parse_ping("Request timed out.\nRequest timed out.", 2);
        assert_eq!(stats.received(), 0);
        assert!(stats.avg_ms().is_none());
        // No summary line parsed -> computed from counts.
        assert_eq!(stats.packet_loss_pct, 100.0);
    }

    #[test]
    fn sub_millisecond_reply_parses() {
        let stats = parse_ping("Reply from 192.168.1.1: bytes=32 time<1ms TTL=64", 1);
        assert_eq!(stats.received(), 1);
        assert_eq!(stats.times_ms[0], 1.0);
    }

    #[test]
    fn merged_bursts_recompute_loss() {
        let first = PingStats::all_lost(3);
        let second = parse_ping("64 bytes from 1.1.1.1: icmp_seq=1 ttl=58 time=10.0 ms", 3);
        let merged = first.merged_with(second);
        assert_eq!(merged.sent, 6);
        assert_eq!(merged.received(), 1);
        assert!((merged.packet_loss_pct - 83.333).abs() < 0.01);
    }
}
