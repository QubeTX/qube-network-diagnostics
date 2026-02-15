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
const NEXTDNS_V4: [&str; 2] = ["45.90.28.0", "45.90.30.0"];

#[allow(dead_code)]
const CLOUDFLARE_V6: &str = "2606:4700:4700::1111";
#[allow(dead_code)]
const GOOGLE_V6: &str = "2001:4860:4860::8888";

// ── DnsProvider enum ─────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq)]
pub enum DnsProvider {
    /// Cloudflare primary + Google secondary
    Hybrid,
    /// Cloudflare 1.1.1.1, 1.0.0.1
    Cloudflare,
    /// Google 8.8.8.8, 8.8.4.4
    Google,
    /// NextDNS encrypted DNS with filtering (config ID)
    NextDns(String),
    /// DHCP-provided (clear manual servers)
    Automatic,
}

impl DnsProvider {
    pub fn servers_v4(&self) -> &[&str] {
        match self {
            DnsProvider::Hybrid => &HYBRID_V4,
            DnsProvider::Cloudflare => &CLOUDFLARE_V4,
            DnsProvider::Google => &GOOGLE_V4,
            DnsProvider::NextDns(_) => &NEXTDNS_V4,
            DnsProvider::Automatic => &[],
        }
    }

    pub fn label(&self) -> &'static str {
        match self {
            DnsProvider::Hybrid => "Hybrid (Cloudflare + Google)",
            DnsProvider::Cloudflare => "Cloudflare (1.1.1.1)",
            DnsProvider::Google => "Google (8.8.8.8)",
            DnsProvider::NextDns(_) => "NextDNS (encrypted)",
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

/// Test TCP reachability of NextDNS (45.90.28.0:53).
pub async fn test_nextdns_reachability() -> bool {
    let timeout = Duration::from_secs(3);
    let result = tokio::time::timeout(
        timeout,
        tokio::net::TcpStream::connect("45.90.28.0:53"),
    )
    .await;
    matches!(result, Ok(Ok(_)))
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
        return chosen;
    }

    match chosen {
        DnsProvider::Hybrid => {
            if cloudflare_ok && google_ok {
                DnsProvider::Hybrid
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
                DnsProvider::Cloudflare
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
                DnsProvider::Google
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
        DnsProvider::NextDns(id) => {
            // NextDNS IPs are always the same; test basic reachability
            if cloudflare_ok || google_ok {
                // If public DNS is reachable, NextDNS should be too
                DnsProvider::NextDns(id)
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
        DnsProvider::Automatic => DnsProvider::Automatic,
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
    if let DnsProvider::NextDns(ref id) = provider {
        // Check if nextdns CLI is installed
        let mut check = tokio::process::Command::new("which");
        check.arg("nextdns");
        let has_cli = if let Ok(output) = run_cmd(check, TIMEOUT_QUICK).await {
            output.status.success()
        } else {
            false
        };

        if has_cli {
            let mut install_cmd = tokio::process::Command::new("nextdns");
            install_cmd.args(["install", "-config", id, "-report-client-info"]);
            if let Ok(output) = run_cmd(install_cmd, TIMEOUT_MEDIUM).await {
                if !output.status.success() {
                    return Err(format!(
                        "nextdns install failed: {}",
                        String::from_utf8_lossy(&output.stderr).trim()
                    ));
                }
            }
            let mut activate_cmd = tokio::process::Command::new("nextdns");
            activate_cmd.arg("activate");
            match run_cmd(activate_cmd, TIMEOUT_MEDIUM).await {
                Ok(output) if output.status.success() => {
                    return Ok(format!("NextDNS activated with config {} on {}", id, service));
                }
                Ok(output) => {
                    return Err(format!(
                        "nextdns activate failed: {}",
                        String::from_utf8_lossy(&output.stderr).trim()
                    ));
                }
                Err(e) => return Err(e),
            }
        } else {
            // No CLI — set DNS IPs directly as fallback (no encryption)
            let servers = provider.servers_v4();
            let mut args: Vec<&str> = vec!["-setdnsservers", service];
            args.extend_from_slice(servers);
            let mut cmd = tokio::process::Command::new("networksetup");
            cmd.args(&args);
            match run_cmd(cmd, TIMEOUT_MEDIUM).await {
                Ok(output) if output.status.success() => {
                    return Ok(format!(
                        "NextDNS IPs set on {} (install nextdns CLI for encrypted DNS: sh -c \"$(curl -sL https://nextdns.io/install)\")",
                        service
                    ));
                }
                Ok(output) => {
                    return Err(format!(
                        "Failed to set DNS: {}",
                        String::from_utf8_lossy(&output.stderr).trim()
                    ));
                }
                Err(e) => return Err(e),
            }
        }
    }

    let label = provider.label();
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
            Ok(format!("DNS set to {} on {}", label, service))
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
    // Register DoH templates for NextDNS before setting IPs
    if let DnsProvider::NextDns(ref id) = provider {
        for ip in &NEXTDNS_V4 {
            let template = format!("https://dns.nextdns.io/{}", id);
            let mut cmd = tokio::process::Command::new("netsh");
            cmd.args([
                "dns", "add", "encryption", &format!("server={}", ip),
                &format!("dohtemplate={}", template),
                "autoupgrade=yes", "udpfallback=no",
            ]);
            let _ = run_cmd(cmd, TIMEOUT_MEDIUM).await; // best-effort
        }
    }

    let label = provider.label();
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

            Ok(format!("DNS set to {} on {}", label, iface))
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

    let label = provider.label();

    if has_resolved {
        // NextDNS with systemd-resolved: configure DoT via resolved.conf
        if let DnsProvider::NextDns(ref id) = provider {
            // Back up existing config
            let mut backup = tokio::process::Command::new("cp");
            backup.args(["/etc/systemd/resolved.conf", "/etc/systemd/resolved.conf.bak"]);
            let _ = run_cmd(backup, TIMEOUT_QUICK).await;

            let config_content = format!(
                "[Resolve]\nDNS=45.90.28.0#{id}.dns.nextdns.io 45.90.30.0#{id}.dns.nextdns.io\n\
                 DNSOverTLS=yes\n",
                id = id,
            );
            if let Err(e) = tokio::fs::write("/etc/systemd/resolved.conf", &config_content).await {
                return Err(format!("Failed to write resolved.conf: {}", e));
            }

            let mut restart = tokio::process::Command::new("systemctl");
            restart.args(["restart", "systemd-resolved"]);
            match run_cmd(restart, TIMEOUT_MEDIUM).await {
                Ok(output) if output.status.success() => {
                    return Ok(format!("NextDNS (DoT) configured on {}", iface));
                }
                _ => return Err("Failed to restart systemd-resolved".to_string()),
            }
        }

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
                        Ok(format!("DNS set to {} on {}", label, iface))
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
        // Fallback: nmcli (no DoT for NextDNS — just set IPs)
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
                        Ok(format!("DNS set to {} on {}", label, iface))
                    }
                    _ => Err("Failed to set DNS via nmcli".to_string()),
                }
            }
        }
    }
}
