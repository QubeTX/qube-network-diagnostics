use std::time::Duration;

use crate::config::Config;
use crate::render::color;

#[allow(unused_imports)]
use super::cmd::{run_cmd, TIMEOUT_MEDIUM, TIMEOUT_QUICK};
use crate::actions::is_interactive;

// ── DNS server constants ─────────────────────────────────────────────────────

const CLOUDFLARE_V4: [&str; 2] = ["1.1.1.1", "1.0.0.1"];
const GOOGLE_V4: [&str; 2] = ["8.8.8.8", "8.8.4.4"];
const HYBRID_V4: [&str; 2] = ["1.1.1.1", "8.8.8.8"];

#[allow(dead_code)]
const CLOUDFLARE_V6: &str = "2606:4700:4700::1111";
#[allow(dead_code)]
const GOOGLE_V6: &str = "2001:4860:4860::8888";

// ── DnsProvider enum ─────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum DnsProvider {
    /// Cloudflare primary + Google secondary
    Hybrid,
    /// Cloudflare 1.1.1.1, 1.0.0.1
    Cloudflare,
    /// Google 8.8.8.8, 8.8.4.4
    Google,
    /// DHCP-provided (clear manual servers)
    Automatic,
}

impl DnsProvider {
    pub fn servers_v4(&self) -> &[&str] {
        match self {
            DnsProvider::Hybrid => &HYBRID_V4,
            DnsProvider::Cloudflare => &CLOUDFLARE_V4,
            DnsProvider::Google => &GOOGLE_V4,
            DnsProvider::Automatic => &[],
        }
    }

    pub fn label(&self) -> &'static str {
        match self {
            DnsProvider::Hybrid => "Hybrid (Cloudflare + Google)",
            DnsProvider::Cloudflare => "Cloudflare (1.1.1.1)",
            DnsProvider::Google => "Google (8.8.8.8)",
            DnsProvider::Automatic => "Automatic (DHCP)",
        }
    }
}

// ── DNS verification ─────────────────────────────────────────────────────────

/// Resolve 3 domains to verify DNS is working. Pass if >= 2 of 3 succeed.
pub async fn verify_dns() -> bool {
    let domains = ["www.google.com:80", "www.cloudflare.com:80", "www.apple.com:80"];
    let timeout = Duration::from_secs(3);

    let mut successes = 0u32;
    for domain in &domains {
        let result = tokio::time::timeout(timeout, tokio::net::lookup_host(domain)).await;
        if let Ok(Ok(mut addrs)) = result {
            if addrs.next().is_some() {
                successes += 1;
            }
        }
    }

    successes >= 2
}

/// Test TCP reachability of Cloudflare (1.1.1.1:53) and Google (8.8.8.8:53).
/// Returns (cloudflare_ok, google_ok). Catches corporate firewalls blocking public DNS.
pub async fn test_dns_reachability() -> (bool, bool) {
    let timeout = Duration::from_secs(3);

    let cf = tokio::time::timeout(
        timeout,
        tokio::net::TcpStream::connect("1.1.1.1:53"),
    );
    let google = tokio::time::timeout(
        timeout,
        tokio::net::TcpStream::connect("8.8.8.8:53"),
    );

    let (cf_result, google_result) = tokio::join!(cf, google);
    let cloudflare_ok = matches!(cf_result, Ok(Ok(_)));
    let google_ok = matches!(google_result, Ok(Ok(_)));

    (cloudflare_ok, google_ok)
}

// ── DNS provider selection ───────────────────────────────────────────────────

/// Prompt the user to choose a DNS provider (Stage 2 only).
/// In non-interactive/JSON mode, returns Hybrid without prompting.
pub fn prompt_dns_choice(config: &Config) -> DnsProvider {
    if !is_interactive(config) {
        return DnsProvider::Hybrid;
    }

    println!();
    println!(
        "  {} {}",
        color::yellow(super::warn_icon(config), config),
        color::yellow("DNS is not resolving correctly. Choose a DNS server:", config),
    );
    println!("    1. Hybrid — Cloudflare + Google (recommended)");
    println!("    2. Cloudflare (1.1.1.1) — privacy-focused");
    println!("    3. Google (8.8.8.8) — reliability");
    println!("    4. Automatic — DHCP-provided");

    use std::io::Write;
    print!("  Choose [1-4, default=1]: ");
    let _ = std::io::stdout().flush();

    let mut input = String::new();
    if std::io::stdin().read_line(&mut input).is_ok() {
        match input.trim() {
            "2" => DnsProvider::Cloudflare,
            "3" => DnsProvider::Google,
            "4" => DnsProvider::Automatic,
            _ => DnsProvider::Hybrid, // "1", empty, or anything else
        }
    } else {
        DnsProvider::Hybrid
    }
}

/// Pick the best provider given reachability results.
/// If the chosen provider's servers are unreachable, fall back to whichever works.
pub fn adjust_for_reachability(
    chosen: DnsProvider,
    cloudflare_ok: bool,
    google_ok: bool,
    config: &Config,
) -> DnsProvider {
    if chosen == DnsProvider::Automatic {
        return chosen; // no servers to test
    }

    match chosen {
        DnsProvider::Hybrid => {
            if cloudflare_ok && google_ok {
                chosen
            } else if cloudflare_ok {
                if is_interactive(config) {
                    println!(
                        "    {}",
                        color::dim("Google DNS (8.8.8.8) unreachable — using Cloudflare only", config),
                    );
                }
                DnsProvider::Cloudflare
            } else if google_ok {
                if is_interactive(config) {
                    println!(
                        "    {}",
                        color::dim("Cloudflare DNS (1.1.1.1) unreachable — using Google only", config),
                    );
                }
                DnsProvider::Google
            } else {
                if is_interactive(config) {
                    println!(
                        "    {}",
                        color::dim("Public DNS servers unreachable — falling back to DHCP", config),
                    );
                }
                DnsProvider::Automatic
            }
        }
        DnsProvider::Cloudflare => {
            if cloudflare_ok {
                chosen
            } else if google_ok {
                if is_interactive(config) {
                    println!(
                        "    {}",
                        color::dim("Cloudflare unreachable — falling back to Google DNS", config),
                    );
                }
                DnsProvider::Google
            } else {
                if is_interactive(config) {
                    println!(
                        "    {}",
                        color::dim("Public DNS servers unreachable — falling back to DHCP", config),
                    );
                }
                DnsProvider::Automatic
            }
        }
        DnsProvider::Google => {
            if google_ok {
                chosen
            } else if cloudflare_ok {
                if is_interactive(config) {
                    println!(
                        "    {}",
                        color::dim("Google unreachable — falling back to Cloudflare DNS", config),
                    );
                }
                DnsProvider::Cloudflare
            } else {
                if is_interactive(config) {
                    println!(
                        "    {}",
                        color::dim("Public DNS servers unreachable — falling back to DHCP", config),
                    );
                }
                DnsProvider::Automatic
            }
        }
        DnsProvider::Automatic => chosen,
    }
}

// ── DNS server configuration ─────────────────────────────────────────────────

/// Set DNS servers on the given interface/service. Platform-specific.
///
/// - `iface`: the BSD-level interface name (e.g. "en0", "eth0", "Wi-Fi")
/// - `service_name`: macOS network service name (e.g. "Wi-Fi"). Ignored on other platforms.
/// - `provider`: which DNS servers to set
pub async fn set_dns_servers(
    iface: &str,
    service_name: &str,
    provider: DnsProvider,
) -> Result<String, String> {
    #[cfg(target_os = "macos")]
    {
        set_dns_macos(service_name, provider).await
    }

    #[cfg(windows)]
    {
        let _ = service_name;
        set_dns_windows(iface, provider).await
    }

    #[cfg(target_os = "linux")]
    {
        let _ = service_name;
        set_dns_linux(iface, provider).await
    }
}

#[cfg(target_os = "macos")]
async fn set_dns_macos(service: &str, provider: DnsProvider) -> Result<String, String> {
    let mut cmd = tokio::process::Command::new("networksetup");
    match provider {
        DnsProvider::Automatic => {
            cmd.args(["-setdnsservers", service, "empty"]);
        }
        _ => {
            let servers = provider.servers_v4();
            let mut args: Vec<&str> = vec!["-setdnsservers", service];
            args.extend_from_slice(servers);
            cmd.args(&args);
        }
    }

    match run_cmd(cmd, TIMEOUT_MEDIUM).await {
        Ok(output) if output.status.success() => {
            Ok(format!("DNS set to {} on {}", provider.label(), service))
        }
        Ok(output) => Err(format!(
            "Failed to set DNS: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )),
        Err(e) => Err(e),
    }
}

#[cfg(windows)]
async fn set_dns_windows(iface: &str, provider: DnsProvider) -> Result<String, String> {
    match provider {
        DnsProvider::Automatic => {
            let mut cmd = tokio::process::Command::new("netsh");
            cmd.args(["interface", "ip", "set", "dns", iface, "dhcp"]);
            match run_cmd(cmd, TIMEOUT_MEDIUM).await {
                Ok(output) if output.status.success() => {
                    Ok(format!("DNS set to DHCP on {}", iface))
                }
                Ok(output) => Err(format!(
                    "Failed to set DNS: {}",
                    String::from_utf8_lossy(&output.stderr).trim()
                )),
                Err(e) => Err(e),
            }
        }
        _ => {
            let servers = provider.servers_v4();
            // Set primary
            let mut cmd1 = tokio::process::Command::new("netsh");
            cmd1.args([
                "interface", "ip", "set", "dns", iface, "static", servers[0],
            ]);
            match run_cmd(cmd1, TIMEOUT_MEDIUM).await {
                Ok(output) if output.status.success() => {}
                Ok(output) => {
                    return Err(format!(
                        "Failed to set primary DNS: {}",
                        String::from_utf8_lossy(&output.stderr).trim()
                    ));
                }
                Err(e) => return Err(e),
            }

            // Add secondary
            if servers.len() > 1 {
                let mut cmd2 = tokio::process::Command::new("netsh");
                cmd2.args([
                    "interface", "ip", "add", "dns", iface, servers[1], "index=2",
                ]);
                let _ = run_cmd(cmd2, TIMEOUT_MEDIUM).await; // best-effort
            }

            Ok(format!("DNS set to {} on {}", provider.label(), iface))
        }
    }
}

#[cfg(target_os = "linux")]
async fn set_dns_linux(iface: &str, provider: DnsProvider) -> Result<String, String> {
    // Check if systemd-resolved is available
    let mut check = tokio::process::Command::new("systemctl");
    check.args(["is-active", "systemd-resolved"]);
    let has_resolved = if let Ok(output) = run_cmd(check, TIMEOUT_QUICK).await {
        String::from_utf8_lossy(&output.stdout).trim() == "active"
    } else {
        false
    };

    if has_resolved {
        match provider {
            DnsProvider::Automatic => {
                let mut cmd = tokio::process::Command::new("resolvectl");
                cmd.args(["revert", iface]);
                match run_cmd(cmd, TIMEOUT_MEDIUM).await {
                    Ok(output) if output.status.success() => {
                        Ok(format!("DNS reverted to DHCP on {}", iface))
                    }
                    Ok(output) => Err(format!(
                        "Failed to revert DNS: {}",
                        String::from_utf8_lossy(&output.stderr).trim()
                    )),
                    Err(e) => Err(e),
                }
            }
            _ => {
                let servers = provider.servers_v4();
                let mut cmd = tokio::process::Command::new("resolvectl");
                let mut args: Vec<&str> = vec!["dns", iface];
                args.extend_from_slice(servers);
                cmd.args(&args);
                match run_cmd(cmd, TIMEOUT_MEDIUM).await {
                    Ok(output) if output.status.success() => {
                        Ok(format!("DNS set to {} on {}", provider.label(), iface))
                    }
                    Ok(output) => Err(format!(
                        "Failed to set DNS: {}",
                        String::from_utf8_lossy(&output.stderr).trim()
                    )),
                    Err(e) => Err(e),
                }
            }
        }
    } else {
        // Fallback: nmcli
        let nm_iface = iface.to_string();
        match provider {
            DnsProvider::Automatic => {
                let mut cmd = tokio::process::Command::new("nmcli");
                cmd.args(["con", "mod", &nm_iface, "ipv4.dns", "", "ipv4.ignore-auto-dns", "no"]);
                match run_cmd(cmd, TIMEOUT_MEDIUM).await {
                    Ok(output) if output.status.success() => {
                        Ok(format!("DNS reverted to DHCP on {}", iface))
                    }
                    _ => Err("Failed to revert DNS via nmcli".to_string()),
                }
            }
            _ => {
                let servers = provider.servers_v4().join(",");
                let mut cmd = tokio::process::Command::new("nmcli");
                cmd.args(["con", "mod", &nm_iface, "ipv4.dns", &servers, "ipv4.ignore-auto-dns", "yes"]);
                match run_cmd(cmd, TIMEOUT_MEDIUM).await {
                    Ok(output) if output.status.success() => {
                        // Apply changes
                        let mut apply = tokio::process::Command::new("nmcli");
                        apply.args(["con", "up", &nm_iface]);
                        let _ = run_cmd(apply, TIMEOUT_MEDIUM).await;
                        Ok(format!("DNS set to {} on {}", provider.label(), iface))
                    }
                    _ => Err("Failed to set DNS via nmcli".to_string()),
                }
            }
        }
    }
}
