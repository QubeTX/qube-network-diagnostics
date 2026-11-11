//! Clock synchronization check — deep diagnostic.
//!
//! A skewed system clock breaks TLS certificate validation in ways that look
//! exactly like a broken network. This module measures the offset with a
//! dependency-free SNTP exchange (48-byte mode-3 packet over UDP/123) against
//! three public servers, taking the median of the responders — identical on
//! every platform, no `w32tm`/`sntp`/`chronyd` needed. When UDP/123 is
//! blocked (all servers silent — itself a finding), an HTTP `Date` header
//! fallback distinguishes "clock fine, NTP blocked" from "clock skewed".

use serde::Serialize;
use std::time::Duration;

use super::util;

const NTP_SERVERS: &[&str] = &["pool.ntp.org", "time.cloudflare.com", "time.google.com"];

/// Per-server receive budget.
const RECV_TIMEOUT: Duration = Duration::from_secs(5);

/// Seconds between the NTP era (1900-01-01) and the Unix epoch (1970-01-01).
const NTP_UNIX_OFFSET_SECS: u64 = 2_208_988_800;

#[derive(Debug, Clone, Serialize)]
pub struct ClockSync {
    /// System clock offset in milliseconds (positive = local clock ahead).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub offset_ms: Option<f64>,
    pub servers_responded: u8,
    /// All SNTP exchanges timed out — UDP/123 is likely filtered.
    pub ntp_blocked: bool,
    /// "sntp" or "http-date".
    pub source: String,
    pub assessment: String,
    pub level: String,
}

pub async fn collect() -> Option<ClockSync> {
    // Query all servers concurrently; each exchange is individually bounded.
    let offsets: Vec<f64> =
        futures_util::future::join_all(NTP_SERVERS.iter().map(|server| sntp_offset_ms(server)))
            .await
            .into_iter()
            .flatten()
            .collect();

    if !offsets.is_empty() {
        let offset = median(&offsets);
        let (assessment, level) = classify_offset(Some(offset), false);
        return Some(ClockSync {
            offset_ms: Some(offset),
            servers_responded: offsets.len() as u8,
            ntp_blocked: false,
            source: "sntp".to_string(),
            assessment,
            level,
        });
    }

    // UDP/123 blocked or all pools unreachable: coarse fallback via an HTTPS
    // Date header (±1s granularity — enough to tell "fine" from "broken").
    let http_offset = http_date_offset_ms().await;
    let ntp_blocked = true;
    let (assessment, level) = classify_offset(http_offset, ntp_blocked);
    Some(ClockSync {
        offset_ms: http_offset,
        servers_responded: 0,
        ntp_blocked,
        source: "http-date".to_string(),
        assessment,
        level,
    })
}

/// One SNTP exchange: offset = ((T2−T1)+(T3−T4))/2 in milliseconds.
async fn sntp_offset_ms(server: &str) -> Option<f64> {
    let addrs = util::lookup_host_timeout(format!("{}:123", server), util::RESOLVE).await?;
    let addr = addrs.first()?;

    let socket = tokio::net::UdpSocket::bind("0.0.0.0:0").await.ok()?;
    socket.connect(addr).await.ok()?;

    // Mode 3 (client), version 4.
    let mut packet = [0u8; 48];
    packet[0] = 0b00_100_011;

    let t1 = unix_now_secs_f64();
    socket.send(&packet).await.ok()?;

    let mut buf = [0u8; 48];
    let n = tokio::time::timeout(RECV_TIMEOUT, socket.recv(&mut buf))
        .await
        .ok()?
        .ok()?;
    let t4 = unix_now_secs_f64();
    if n < 48 {
        return None;
    }

    let t2 = parse_ntp_timestamp(&buf[32..40])?; // receive timestamp
    let t3 = parse_ntp_timestamp(&buf[40..48])?; // transmit timestamp

    // offset = ((T2 - T1) + (T3 - T4)) / 2; positive = server ahead of us,
    // so the LOCAL clock offset is the negation.
    let offset_secs = ((t2 - t1) + (t3 - t4)) / 2.0;
    let local_offset_ms = -offset_secs * 1000.0;
    local_offset_ms.is_finite().then_some(local_offset_ms)
}

fn unix_now_secs_f64() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs_f64())
        .unwrap_or(0.0)
}

/// 64-bit NTP timestamp (32.32 fixed point, era 1900) → Unix seconds.
fn parse_ntp_timestamp(bytes: &[u8]) -> Option<f64> {
    if bytes.len() < 8 {
        return None;
    }
    let secs = u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]) as u64;
    let frac = u32::from_be_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]) as f64;
    if secs == 0 {
        return None;
    }
    let unix_secs = secs.checked_sub(NTP_UNIX_OFFSET_SECS)? as f64;
    Some(unix_secs + frac / (u32::MAX as f64 + 1.0))
}

/// Coarse offset from an HTTPS response's Date header (±1s).
async fn http_date_offset_ms() -> Option<f64> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()
        .ok()?;
    let resp = client
        .get("https://1.1.1.1/cdn-cgi/trace")
        .send()
        .await
        .ok()?;
    let date = resp.headers().get("date")?.to_str().ok()?;
    let server_time = httpdate_to_unix(date)?;
    Some((unix_now_secs_f64() - server_time) * 1000.0)
}

/// Parse an RFC 7231 HTTP date ("Tue, 10 Jun 2026 12:34:56 GMT") to Unix
/// seconds. Hand-rolled to avoid a date-crate dependency.
fn httpdate_to_unix(s: &str) -> Option<f64> {
    let parts: Vec<&str> = s.split_whitespace().collect();
    // ["Tue,", "10", "Jun", "2026", "12:34:56", "GMT"]
    if parts.len() != 6 {
        return None;
    }
    let day: i64 = parts[1].parse().ok()?;
    let month = match parts[2] {
        "Jan" => 1,
        "Feb" => 2,
        "Mar" => 3,
        "Apr" => 4,
        "May" => 5,
        "Jun" => 6,
        "Jul" => 7,
        "Aug" => 8,
        "Sep" => 9,
        "Oct" => 10,
        "Nov" => 11,
        "Dec" => 12,
        _ => return None,
    };
    let year: i64 = parts[3].parse().ok()?;
    let hms: Vec<&str> = parts[4].split(':').collect();
    if hms.len() != 3 {
        return None;
    }
    let (h, m, sec): (i64, i64, i64) = (
        hms[0].parse().ok()?,
        hms[1].parse().ok()?,
        hms[2].parse().ok()?,
    );

    // Days since Unix epoch via the standard civil-date algorithm.
    let y = if month <= 2 { year - 1 } else { year };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let mp = (month + 9) % 12;
    let doy = (153 * mp + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;

    Some((days * 86_400 + h * 3_600 + m * 60 + sec) as f64)
}

fn median(values: &[f64]) -> f64 {
    let mut sorted = values.to_vec();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    sorted[sorted.len() / 2]
}

/// Classification — pure, unit-testable.
fn classify_offset(offset_ms: Option<f64>, ntp_blocked: bool) -> (String, String) {
    match offset_ms {
        Some(ms) if ms.abs() > 30_000.0 => (
            format!(
                "Clock is off by {:.0}s — TLS certificate validation will fail; fix the system time",
                ms / 1000.0
            ),
            "fail".to_string(),
        ),
        Some(ms) if ms.abs() > 1_000.0 => (
            format!("Clock is off by {:.1}s — time sync is drifting", ms / 1000.0),
            "warn".to_string(),
        ),
        Some(_) if ntp_blocked => (
            "Clock is accurate, but NTP (UDP/123) appears blocked — drift won't self-correct"
                .to_string(),
            "warn".to_string(),
        ),
        Some(_) => ("Clock is synchronized".to_string(), "ok".to_string()),
        None => (
            "Clock offset could not be measured (NTP blocked, no HTTP fallback)".to_string(),
            "warn".to_string(),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ntp_timestamp_roundtrip() {
        // 2026-01-01 00:00:00 UTC = Unix 1767225600.
        let unix: u64 = 1_767_225_600;
        let ntp_secs = (unix + NTP_UNIX_OFFSET_SECS) as u32;
        let mut bytes = [0u8; 8];
        bytes[..4].copy_from_slice(&ntp_secs.to_be_bytes());
        // Half-second fraction.
        bytes[4..].copy_from_slice(&(u32::MAX / 2 + 1).to_be_bytes());
        let parsed = parse_ntp_timestamp(&bytes).unwrap();
        assert!((parsed - (unix as f64 + 0.5)).abs() < 0.001, "got {parsed}");
    }

    #[test]
    fn zero_timestamp_rejected() {
        assert!(parse_ntp_timestamp(&[0u8; 8]).is_none());
    }

    #[test]
    fn httpdate_parses_rfc7231() {
        // Known value: 2026-06-10 12:00:00 UTC = 1781092800
        // (2026-01-01 = 1767225600, +160 days, +12h).
        let unix = httpdate_to_unix("Wed, 10 Jun 2026 12:00:00 GMT").unwrap();
        assert_eq!(unix, 1_781_092_800.0);
        assert!(httpdate_to_unix("garbage").is_none());
    }

    #[test]
    fn classify_offset_thresholds() {
        assert_eq!(classify_offset(Some(100.0), false).1, "ok");
        assert_eq!(classify_offset(Some(5_000.0), false).1, "warn");
        assert_eq!(classify_offset(Some(-5_000.0), false).1, "warn");
        assert_eq!(classify_offset(Some(60_000.0), false).1, "fail");
        assert_eq!(classify_offset(Some(-60_000.0), false).1, "fail");
        assert_eq!(classify_offset(Some(100.0), true).1, "warn");
        assert_eq!(classify_offset(None, true).1, "warn");
    }

    #[test]
    fn median_of_offsets() {
        assert_eq!(median(&[5.0, -3.0, 2.0]), 2.0);
        assert_eq!(median(&[7.0]), 7.0);
    }
}
