use crate::config::{Config, OutputFormat};
use crate::render::color;
use crate::render::progress::create_spinner;

use super::fix::dns as fix_dns;
use super::fix::dns::DnsProvider;
use super::fix::{print_step_fail, print_step_ok};
use super::{
    fail_icon, flush_dns_platform, is_interactive, print_elevation_hint, prompt_string,
    success_icon,
};

/// Interactive DNS change flow.
pub async fn run(config: &Config) -> i32 {
    if config.format == OutputFormat::Json {
        return run_json(config).await;
    }

    println!();

    // Elevation check
    if !crate::platform::is_elevated() {
        println!();
        println!(
            "  {}",
            color::red("Changing DNS requires elevated privileges.", config),
        );
        print_elevation_hint(config);
        return 2;
    }

    // Detect default interface
    let iface = match super::fix::adapters::detect_default_interface().await {
        Some(i) => i,
        None => {
            println!(
                "  {} {}",
                color::red(fail_icon(config), config),
                color::red("Could not detect default network interface", config),
            );
            return 2;
        }
    };

    // Detect service name (macOS needs this, other platforms use iface)
    let service_name = super::fix::stages::detect_service_name(&iface).await;

    println!(
        "  Interface: {}",
        color::cyan(&iface, config),
    );
    println!();

    // Prompt for DNS choice (extended with NextDNS)
    let provider = prompt_dns_choice_extended(config);

    // Handle NextDNS config ID
    let provider = if let DnsProvider::NextDns(_) = &provider {
        let id = prompt_string("  Enter NextDNS config ID (e.g. 7915d6): ");
        if id.is_empty() {
            println!(
                "  {}",
                color::red("No config ID provided — aborting.", config),
            );
            return 2;
        }
        DnsProvider::NextDns(id)
    } else {
        provider
    };

    // Test reachability (skip for Automatic)
    let provider = if provider != DnsProvider::Automatic {
        let spinner = create_spinner("Testing DNS server reachability...");
        let (cf_ok, google_ok) = fix_dns::test_dns_reachability().await;

        // For NextDNS, also test NextDNS-specific reachability
        if let DnsProvider::NextDns(_) = &provider {
            let nextdns_ok = fix_dns::test_nextdns_reachability().await;
            spinner.finish_and_clear();
            if !nextdns_ok {
                println!(
                    "    {}",
                    color::dim("NextDNS servers unreachable — falling back", config),
                );
                fix_dns::adjust_for_reachability(DnsProvider::Hybrid, cf_ok, google_ok, config)
            } else {
                provider
            }
        } else {
            spinner.finish_and_clear();
            fix_dns::adjust_for_reachability(provider, cf_ok, google_ok, config)
        }
    } else {
        provider
    };

    // Set DNS servers
    let spinner = create_spinner(&format!("Setting DNS to {}...", provider.label()));
    let result = fix_dns::set_dns_servers(&iface, &service_name, provider.clone()).await;
    spinner.finish_and_clear();

    match &result {
        Ok(msg) => print_step_ok(msg, config),
        Err(msg) => {
            print_step_fail("Failed to set DNS servers", msg, config);
            return 2;
        }
    }

    // Flush DNS cache
    let spinner = create_spinner("Flushing DNS cache...");
    let _ = flush_dns_platform().await;
    spinner.finish_and_clear();

    // Wait for propagation
    let spinner = create_spinner("Waiting for DNS to propagate...");
    tokio::time::sleep(std::time::Duration::from_secs(5)).await;
    spinner.finish_and_clear();

    // Verify
    let spinner = create_spinner("Verifying DNS resolution...");
    let dns_ok = fix_dns::verify_dns().await;
    spinner.finish_and_clear();

    let http_ok = if dns_ok {
        let spinner = create_spinner("Checking internet connectivity...");
        let ok = super::fix::connectivity::check_connectivity().await;
        spinner.finish_and_clear();
        ok
    } else {
        false
    };

    if dns_ok && http_ok {
        print_step_ok("DNS resolution and connectivity verified", config);
        println!();
        println!(
            "  {} {}",
            color::green(success_icon(config), config),
            color::green("DNS configured successfully — running diagnostics...", config),
        );
        println!();
        return 0; // fall through to diagnostics
    }

    // Verification failed — auto-revert if not Automatic
    if provider != DnsProvider::Automatic {
        println!(
            "  {} {}",
            color::yellow(super::fix::warn_icon(config), config),
            color::yellow("Verification failed — reverting to automatic DNS...", config),
        );

        // Platform-specific NextDNS revert
        #[cfg(target_os = "linux")]
        if matches!(provider, DnsProvider::NextDns(_)) {
            // Restore backed-up resolved.conf if it exists
            let mut restore = tokio::process::Command::new("cp");
            restore.args(["/etc/systemd/resolved.conf.bak", "/etc/systemd/resolved.conf"]);
            let _ = super::fix::cmd::run_cmd(restore, super::fix::cmd::TIMEOUT_QUICK).await;
            let mut restart = tokio::process::Command::new("systemctl");
            restart.args(["restart", "systemd-resolved"]);
            let _ = super::fix::cmd::run_cmd(restart, super::fix::cmd::TIMEOUT_MEDIUM).await;
        }

        #[cfg(target_os = "macos")]
        if matches!(provider, DnsProvider::NextDns(_)) {
            // Deactivate NextDNS CLI if installed
            let mut deactivate = tokio::process::Command::new("nextdns");
            deactivate.arg("deactivate");
            let _ = super::fix::cmd::run_cmd(deactivate, super::fix::cmd::TIMEOUT_MEDIUM).await;
        }

        // Revert to DHCP
        let spinner = create_spinner("Reverting to automatic DNS...");
        let revert = fix_dns::set_dns_servers(&iface, &service_name, DnsProvider::Automatic).await;
        spinner.finish_and_clear();

        match &revert {
            Ok(msg) => print_step_ok(msg, config),
            Err(msg) => {
                print_step_fail("Failed to revert DNS", msg, config);
                return 2;
            }
        }

        // Flush and re-verify
        let _ = flush_dns_platform().await;
        let spinner = create_spinner("Waiting for DNS to propagate...");
        tokio::time::sleep(std::time::Duration::from_secs(5)).await;
        spinner.finish_and_clear();

        let spinner = create_spinner("Re-verifying DNS resolution...");
        let dns_ok = fix_dns::verify_dns().await;
        spinner.finish_and_clear();

        if dns_ok {
            print_step_ok("DNS resolution restored after revert", config);
            println!();
            println!(
                "  {} {}",
                color::yellow(super::fix::warn_icon(config), config),
                color::yellow(
                    "Reverted to automatic DNS — running diagnostics...",
                    config,
                ),
            );
            println!();
            return 0;
        }
    }

    println!();
    println!(
        "  {} {}",
        color::red(fail_icon(config), config),
        color::red("DNS change failed — connectivity could not be verified", config),
    );
    2
}

/// JSON mode: auto-selects Hybrid (no NextDNS — requires interactive config ID).
async fn run_json(config: &Config) -> i32 {
    if !crate::platform::is_elevated() {
        let output = serde_json::json!({
            "action": "dns",
            "error": "elevated_privileges_required",
            "message": "Run with sudo (Unix) or as Administrator (Windows).",
        });
        println!(
            "{}",
            serde_json::to_string_pretty(&output).unwrap_or_else(|_| "{}".to_string())
        );
        return 2;
    }

    let iface = match super::fix::adapters::detect_default_interface().await {
        Some(i) => i,
        None => {
            let output = serde_json::json!({
                "action": "dns",
                "error": "no_interface",
                "message": "Could not detect default network interface.",
            });
            println!(
                "{}",
                serde_json::to_string_pretty(&output).unwrap_or_else(|_| "{}".to_string())
            );
            return 2;
        }
    };

    let service_name = super::fix::stages::detect_service_name(&iface).await;

    // Test reachability
    let (cf_ok, google_ok) = fix_dns::test_dns_reachability().await;
    let provider = fix_dns::adjust_for_reachability(DnsProvider::Hybrid, cf_ok, google_ok, config);

    // Set DNS
    let set_result = fix_dns::set_dns_servers(&iface, &service_name, provider.clone()).await;
    let set_ok = set_result.is_ok();
    let set_msg = match &set_result {
        Ok(msg) => msg.clone(),
        Err(msg) => msg.clone(),
    };

    if !set_ok {
        let output = serde_json::json!({
            "action": "dns",
            "interface": iface,
            "provider": provider.label(),
            "success": false,
            "message": set_msg,
        });
        println!(
            "{}",
            serde_json::to_string_pretty(&output).unwrap_or_else(|_| "{}".to_string())
        );
        return 2;
    }

    // Flush + wait + verify
    let _ = flush_dns_platform().await;
    tokio::time::sleep(std::time::Duration::from_secs(5)).await;
    let dns_ok = fix_dns::verify_dns().await;
    let http_ok = if dns_ok {
        super::fix::connectivity::check_connectivity().await
    } else {
        false
    };

    let verified = dns_ok && http_ok;

    // Auto-revert on failure
    let mut reverted = false;
    if !verified {
        let _ = fix_dns::set_dns_servers(&iface, &service_name, DnsProvider::Automatic).await;
        let _ = flush_dns_platform().await;
        tokio::time::sleep(std::time::Duration::from_secs(5)).await;
        reverted = true;
    }

    let output = serde_json::json!({
        "action": "dns",
        "interface": iface,
        "provider": provider.label(),
        "success": verified,
        "reverted": reverted,
        "message": if verified { set_msg } else { "Verification failed — reverted to DHCP".to_string() },
    });
    println!(
        "{}",
        serde_json::to_string_pretty(&output).unwrap_or_else(|_| "{}".to_string())
    );

    if verified { 0 } else { 2 }
}

/// Extended DNS choice prompt including NextDNS option.
fn prompt_dns_choice_extended(config: &Config) -> DnsProvider {
    if !is_interactive(config) {
        return DnsProvider::Hybrid;
    }

    println!("  Choose a DNS provider:");
    println!("    1. Hybrid — Cloudflare + Google (recommended)");
    println!("    2. Cloudflare (1.1.1.1) — privacy-focused");
    println!("    3. Google (8.8.8.8) — reliability");
    println!("    4. NextDNS — encrypted DNS with filtering");
    println!("    5. Automatic — reset to system default (DHCP)");

    use std::io::Write;
    print!("  Choose [1-5, default=1]: ");
    let _ = std::io::stdout().flush();

    let mut input = String::new();
    if std::io::stdin().read_line(&mut input).is_ok() {
        match input.trim() {
            "2" => DnsProvider::Cloudflare,
            "3" => DnsProvider::Google,
            "4" => DnsProvider::NextDns(String::new()), // placeholder, ID prompted separately
            "5" => DnsProvider::Automatic,
            _ => DnsProvider::Hybrid,
        }
    } else {
        DnsProvider::Hybrid
    }
}
