//! Wi-Fi link quality — deep diagnostic.
//!
//! Structured radio data for the active Wi-Fi link(s): signal strength,
//! channel/band, PHY mode, security, and negotiated rates. The interfaces
//! module shows a one-line summary; this module parses the full picture for
//! technicians. Returns `None` (section omitted) on all-wired machines.

use serde::Serialize;

use super::util;

#[derive(Debug, Clone, Serialize)]
pub struct WifiLink {
    pub interface: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ssid: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bssid: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub signal_pct: Option<u8>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rssi_dbm: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub channel: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub band: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub phy_mode: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub security: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rx_rate_mbps: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tx_rate_mbps: Option<f64>,
    pub assessment: String,
    pub level: String,
}

pub async fn collect() -> Option<Vec<WifiLink>> {
    #[cfg(windows)]
    {
        let mut cmd = tokio::process::Command::new("netsh");
        cmd.args(["wlan", "show", "interfaces"]);
        let output = util::run_with_timeout(cmd, util::QUICK).await?;
        let text = String::from_utf8_lossy(&output.stdout);
        let links = parse_netsh(&text);
        if links.is_empty() {
            None
        } else {
            Some(links)
        }
    }

    #[cfg(target_os = "macos")]
    {
        let mut cmd = tokio::process::Command::new(
            "/System/Library/PrivateFrameworks/Apple80211.framework/Versions/Current/Resources/airport",
        );
        cmd.arg("-I");
        let output = util::run_with_timeout(cmd, util::QUICK).await?;
        let text = String::from_utf8_lossy(&output.stdout);
        let links = parse_airport(&text);
        if links.is_empty() {
            None
        } else {
            Some(links)
        }
    }

    #[cfg(target_os = "linux")]
    {
        // Primary: nmcli terse mode (carries security info).
        let mut cmd = tokio::process::Command::new("nmcli");
        cmd.args([
            "-t",
            "-f",
            "ACTIVE,SSID,BSSID,SIGNAL,CHAN,FREQ,RATE,SECURITY",
            "dev",
            "wifi",
        ]);
        if let Some(output) = util::run_with_timeout(cmd, util::QUICK).await {
            if output.status.success() {
                let text = String::from_utf8_lossy(&output.stdout);
                let links = parse_nmcli(&text);
                if !links.is_empty() {
                    return Some(links);
                }
            }
        }

        // Fallback: iw (no NetworkManager).
        let mut dev_cmd = tokio::process::Command::new("iw");
        dev_cmd.arg("dev");
        let dev_output = util::run_with_timeout(dev_cmd, util::QUICK).await?;
        let dev_text = String::from_utf8_lossy(&dev_output.stdout);
        let ifaces: Vec<String> = dev_text
            .lines()
            .filter_map(|l| l.trim().strip_prefix("Interface "))
            .map(|s| s.to_string())
            .collect();

        let mut links = Vec::new();
        for iface in ifaces {
            let mut link_cmd = tokio::process::Command::new("iw");
            link_cmd.args(["dev", &iface, "link"]);
            if let Some(output) = util::run_with_timeout(link_cmd, util::QUICK).await {
                let text = String::from_utf8_lossy(&output.stdout);
                if let Some(link) = parse_iw_link(&iface, &text) {
                    links.push(link);
                }
            }
        }
        if links.is_empty() {
            None
        } else {
            Some(links)
        }
    }
}

/// Signal assessment shared by every platform parser.
fn assess_signal(
    signal_pct: Option<u8>,
    rssi_dbm: Option<i32>,
    security: Option<&str>,
) -> (String, String) {
    // Open/WEP security overrides signal quality — it's the bigger problem.
    if let Some(sec) = security {
        let lower = sec.to_lowercase();
        if lower.contains("wep") || lower == "open" || lower == "none" || lower == "--" {
            return (
                "Insecure Wi-Fi (open/WEP) — traffic is not protected".to_string(),
                "warn".to_string(),
            );
        }
    }

    let strong = signal_pct.is_some_and(|s| s >= 60) || rssi_dbm.is_some_and(|r| r >= -60);
    let weak = signal_pct.is_some_and(|s| s < 40) || rssi_dbm.is_some_and(|r| r < -70);

    if weak {
        (
            "Weak signal — move closer to the access point or check channel congestion".to_string(),
            "fail".to_string(),
        )
    } else if strong {
        ("Strong signal".to_string(), "ok".to_string())
    } else if signal_pct.is_some() || rssi_dbm.is_some() {
        (
            "Fair signal — workable, but expect reduced throughput".to_string(),
            "warn".to_string(),
        )
    } else {
        ("Signal strength unavailable".to_string(), "ok".to_string())
    }
}

/// Frequency (MHz) → band label.
#[cfg(any(target_os = "linux", test))]
fn band_from_mhz(mhz: u32) -> &'static str {
    if mhz >= 5925 {
        "6 GHz"
    } else if mhz >= 4900 {
        "5 GHz"
    } else {
        "2.4 GHz"
    }
}

/// Parse `netsh wlan show interfaces` — one block per interface.
#[cfg(any(windows, test))]
fn parse_netsh(text: &str) -> Vec<WifiLink> {
    let mut links = Vec::new();
    let mut current: Option<NetshAccum> = None;

    #[derive(Default)]
    struct NetshAccum {
        interface: String,
        ssid: Option<String>,
        bssid: Option<String>,
        signal_pct: Option<u8>,
        channel: Option<u32>,
        band: Option<String>,
        phy_mode: Option<String>,
        security: Option<String>,
        rx_rate_mbps: Option<f64>,
        tx_rate_mbps: Option<f64>,
        connected: bool,
    }

    fn flush(acc: Option<NetshAccum>, links: &mut Vec<WifiLink>) {
        if let Some(a) = acc {
            if a.connected {
                let (assessment, level) = assess_signal(a.signal_pct, None, a.security.as_deref());
                links.push(WifiLink {
                    interface: a.interface,
                    ssid: a.ssid,
                    bssid: a.bssid,
                    signal_pct: a.signal_pct,
                    rssi_dbm: None,
                    channel: a.channel,
                    band: a.band,
                    phy_mode: a.phy_mode,
                    security: a.security,
                    rx_rate_mbps: a.rx_rate_mbps,
                    tx_rate_mbps: a.tx_rate_mbps,
                    assessment,
                    level,
                });
            }
        }
    }

    for line in text.lines() {
        let trimmed = line.trim();
        let Some((key, value)) = trimmed.split_once(':') else {
            continue;
        };
        let key = key.trim();
        let value = value.trim().to_string();
        match key {
            "Name" => {
                flush(current.take(), &mut links);
                current = Some(NetshAccum {
                    interface: value,
                    ..Default::default()
                });
            }
            "State" => {
                if let Some(acc) = current.as_mut() {
                    acc.connected = value.eq_ignore_ascii_case("connected");
                }
            }
            "SSID" => {
                if let Some(acc) = current.as_mut() {
                    if acc.ssid.is_none() && !value.is_empty() {
                        acc.ssid = Some(value);
                    }
                }
            }
            "BSSID" => {
                if let Some(acc) = current.as_mut() {
                    acc.bssid = Some(value);
                }
            }
            "Signal" => {
                if let Some(acc) = current.as_mut() {
                    acc.signal_pct = value.trim_end_matches('%').parse().ok();
                }
            }
            "Channel" => {
                if let Some(acc) = current.as_mut() {
                    acc.channel = value.parse().ok();
                }
            }
            "Band" => {
                if let Some(acc) = current.as_mut() {
                    acc.band = Some(value);
                }
            }
            "Radio type" => {
                if let Some(acc) = current.as_mut() {
                    acc.phy_mode = Some(value);
                }
            }
            "Authentication" => {
                if let Some(acc) = current.as_mut() {
                    acc.security = Some(value);
                }
            }
            "Receive rate (Mbps)" => {
                if let Some(acc) = current.as_mut() {
                    acc.rx_rate_mbps = value.parse().ok();
                }
            }
            "Transmit rate (Mbps)" => {
                if let Some(acc) = current.as_mut() {
                    acc.tx_rate_mbps = value.parse().ok();
                }
            }
            _ => {}
        }
    }
    flush(current, &mut links);
    links
}

/// Parse `airport -I` (macOS, pre-14.4 path).
#[cfg(any(target_os = "macos", test))]
fn parse_airport(text: &str) -> Vec<WifiLink> {
    let mut ssid = None;
    let mut bssid = None;
    let mut rssi: Option<i32> = None;
    let mut channel: Option<u32> = None;
    let mut tx_rate: Option<f64> = None;
    let mut security: Option<String> = None;
    let mut state_running = false;

    for line in text.lines() {
        let Some((key, value)) = line.trim().split_once(':') else {
            continue;
        };
        let key = key.trim();
        let value = value.trim();
        match key {
            "AirPort" => state_running = !value.eq_ignore_ascii_case("Off"),
            "state" => state_running = state_running || value == "running",
            "SSID" => ssid = Some(value.to_string()),
            "BSSID" => bssid = Some(value.to_string()),
            "agrCtlRSSI" => rssi = value.parse().ok(),
            "channel" => {
                // "channel: 36,80" — number before the comma.
                channel = value.split(',').next().and_then(|c| c.trim().parse().ok());
            }
            "lastTxRate" => tx_rate = value.parse().ok(),
            "link auth" => security = Some(value.to_string()),
            _ => {}
        }
    }

    if ssid.is_none() && rssi.is_none() {
        return Vec::new();
    }
    let _ = state_running;

    let band = channel.map(|c| {
        if c > 14 {
            "5 GHz".to_string()
        } else {
            "2.4 GHz".to_string()
        }
    });
    let (assessment, level) = assess_signal(None, rssi, security.as_deref());
    vec![WifiLink {
        interface: "en0".to_string(),
        ssid,
        bssid,
        signal_pct: None,
        rssi_dbm: rssi,
        channel,
        band,
        phy_mode: None,
        security,
        rx_rate_mbps: None,
        tx_rate_mbps: tx_rate,
        assessment,
        level,
    }]
}

/// Parse `nmcli -t -f ACTIVE,SSID,BSSID,SIGNAL,CHAN,FREQ,RATE,SECURITY dev wifi`.
/// Terse mode escapes colons inside values (BSSIDs) as `\:`.
#[cfg(any(target_os = "linux", test))]
fn parse_nmcli(text: &str) -> Vec<WifiLink> {
    let mut links = Vec::new();
    for line in text.lines() {
        let fields = split_terse(line);
        if fields.len() < 8 || fields[0] != "yes" {
            continue;
        }
        let signal: Option<u8> = fields[3].parse().ok();
        let channel: Option<u32> = fields[4].parse().ok();
        let freq_mhz: Option<u32> = fields[5]
            .split_whitespace()
            .next()
            .and_then(|f| f.parse().ok());
        let rate: Option<f64> = fields[6]
            .split_whitespace()
            .next()
            .and_then(|r| r.parse().ok());
        let security = if fields[7].is_empty() {
            Some("open".to_string())
        } else {
            Some(fields[7].clone())
        };
        let (assessment, level) = assess_signal(signal, None, security.as_deref());
        links.push(WifiLink {
            interface: "wifi".to_string(),
            ssid: (!fields[1].is_empty()).then(|| fields[1].clone()),
            bssid: (!fields[2].is_empty()).then(|| fields[2].clone()),
            signal_pct: signal,
            rssi_dbm: None,
            channel,
            band: freq_mhz.map(|f| band_from_mhz(f).to_string()),
            phy_mode: None,
            security,
            rx_rate_mbps: rate,
            tx_rate_mbps: None,
            assessment,
            level,
        });
    }
    links
}

/// Split an nmcli terse line on unescaped colons; `\:` is a literal colon.
#[cfg(any(target_os = "linux", test))]
fn split_terse(line: &str) -> Vec<String> {
    let mut fields = Vec::new();
    let mut current = String::new();
    let mut escaped = false;
    for c in line.chars() {
        if escaped {
            current.push(c);
            escaped = false;
        } else if c == '\\' {
            escaped = true;
        } else if c == ':' {
            fields.push(std::mem::take(&mut current));
        } else {
            current.push(c);
        }
    }
    fields.push(current);
    fields
}

/// Parse `iw dev <iface> link`.
#[cfg(any(target_os = "linux", test))]
fn parse_iw_link(iface: &str, text: &str) -> Option<WifiLink> {
    if text.contains("Not connected") {
        return None;
    }
    let mut ssid = None;
    let mut bssid = None;
    let mut rssi: Option<i32> = None;
    let mut freq_mhz: Option<u32> = None;
    let mut tx_rate: Option<f64> = None;
    let mut rx_rate: Option<f64> = None;

    for line in text.lines() {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix("Connected to ") {
            bssid = rest.split_whitespace().next().map(|s| s.to_string());
        } else if let Some(rest) = trimmed.strip_prefix("SSID:") {
            ssid = Some(rest.trim().to_string());
        } else if let Some(rest) = trimmed.strip_prefix("signal:") {
            rssi = rest.split_whitespace().next().and_then(|v| v.parse().ok());
        } else if let Some(rest) = trimmed.strip_prefix("freq:") {
            // Older iw prints integer MHz; newer prints "5180.0".
            freq_mhz = rest
                .trim()
                .split('.')
                .next()
                .and_then(|v| v.trim().parse().ok());
        } else if let Some(rest) = trimmed.strip_prefix("tx bitrate:") {
            tx_rate = rest.split_whitespace().next().and_then(|v| v.parse().ok());
        } else if let Some(rest) = trimmed.strip_prefix("rx bitrate:") {
            rx_rate = rest.split_whitespace().next().and_then(|v| v.parse().ok());
        }
    }

    if ssid.is_none() && bssid.is_none() {
        return None;
    }

    let (assessment, level) = assess_signal(None, rssi, None);
    Some(WifiLink {
        interface: iface.to_string(),
        ssid,
        bssid,
        signal_pct: None,
        rssi_dbm: rssi,
        channel: None,
        band: freq_mhz.map(|f| band_from_mhz(f).to_string()),
        phy_mode: None,
        security: None,
        rx_rate_mbps: rx_rate,
        tx_rate_mbps: tx_rate,
        assessment,
        level,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const NETSH_FIXTURE: &str = "
There is 1 interface on the system:

    Name                   : Wi-Fi
    Description            : Intel(R) Wi-Fi 6E AX211 160MHz
    State                  : connected
    SSID                   : OttNet-5G
    BSSID                  : aa:bb:cc:dd:ee:ff
    Radio type             : 802.11ax
    Authentication         : WPA2-Personal
    Cipher                 : CCMP
    Band                   : 5 GHz
    Channel                : 44
    Receive rate (Mbps)    : 866.7
    Transmit rate (Mbps)   : 866.7
    Signal                 : 88%
";

    #[test]
    fn parses_netsh_connected_interface() {
        let links = parse_netsh(NETSH_FIXTURE);
        assert_eq!(links.len(), 1);
        let l = &links[0];
        assert_eq!(l.ssid.as_deref(), Some("OttNet-5G"));
        assert_eq!(l.signal_pct, Some(88));
        assert_eq!(l.channel, Some(44));
        assert_eq!(l.band.as_deref(), Some("5 GHz"));
        assert_eq!(l.phy_mode.as_deref(), Some("802.11ax"));
        assert_eq!(l.security.as_deref(), Some("WPA2-Personal"));
        assert_eq!(l.level, "ok");
    }

    #[test]
    fn disconnected_netsh_interface_skipped() {
        let text = "    Name : Wi-Fi\n    State : disconnected\n";
        assert!(parse_netsh(text).is_empty());
    }

    const AIRPORT_FIXTURE: &str = "
     agrCtlRSSI: -52
     agrCtlNoise: -94
          state: running
        lastTxRate: 867
           SSID: OttNet-5G
          BSSID: aa:bb:cc:dd:ee:ff
        channel: 44,80
      link auth: wpa2-psk
";

    #[test]
    fn parses_airport_output() {
        let links = parse_airport(AIRPORT_FIXTURE);
        assert_eq!(links.len(), 1);
        let l = &links[0];
        assert_eq!(l.rssi_dbm, Some(-52));
        assert_eq!(l.channel, Some(44));
        assert_eq!(l.band.as_deref(), Some("5 GHz"));
        assert_eq!(l.level, "ok");
    }

    #[test]
    fn parses_nmcli_terse_with_escaped_bssid() {
        let line = r"yes:OttNet-5G:AA\:BB\:CC\:DD\:EE\:FF:88:44:5220 MHz:405 Mbit/s:WPA2";
        let links = parse_nmcli(line);
        assert_eq!(links.len(), 1);
        let l = &links[0];
        assert_eq!(l.bssid.as_deref(), Some("AA:BB:CC:DD:EE:FF"));
        assert_eq!(l.signal_pct, Some(88));
        assert_eq!(l.band.as_deref(), Some("5 GHz"));
    }

    #[test]
    fn nmcli_inactive_rows_skipped() {
        let line = r"no:OtherNet:AA\:BB\:CC\:DD\:EE\:00:70:1:2412 MHz:130 Mbit/s:WPA2";
        assert!(parse_nmcli(line).is_empty());
    }

    const IW_FIXTURE: &str = "
Connected to aa:bb:cc:dd:ee:ff (on wlan0)
	SSID: OttNet-5G
	freq: 5220
	signal: -55 dBm
	rx bitrate: 433.3 MBit/s
	tx bitrate: 433.3 MBit/s
";

    #[test]
    fn parses_iw_link() {
        let l = parse_iw_link("wlan0", IW_FIXTURE).unwrap();
        assert_eq!(l.rssi_dbm, Some(-55));
        assert_eq!(l.band.as_deref(), Some("5 GHz"));
        assert_eq!(l.ssid.as_deref(), Some("OttNet-5G"));
    }

    #[test]
    fn iw_not_connected_is_none() {
        assert!(parse_iw_link("wlan0", "Not connected.").is_none());
    }

    #[test]
    fn assess_signal_thresholds() {
        assert_eq!(assess_signal(Some(88), None, Some("WPA2")).1, "ok");
        assert_eq!(assess_signal(Some(50), None, Some("WPA2")).1, "warn");
        assert_eq!(assess_signal(Some(30), None, Some("WPA2")).1, "fail");
        assert_eq!(assess_signal(None, Some(-50), Some("WPA2")).1, "ok");
        assert_eq!(assess_signal(None, Some(-65), Some("WPA2")).1, "warn");
        assert_eq!(assess_signal(None, Some(-75), Some("WPA2")).1, "fail");
        // Insecure overrides good signal.
        assert_eq!(assess_signal(Some(95), None, Some("WEP")).1, "warn");
        assert_eq!(assess_signal(Some(95), None, Some("open")).1, "warn");
    }
}
