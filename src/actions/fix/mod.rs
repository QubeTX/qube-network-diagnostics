pub mod adapters;
pub mod arp;
pub mod cmd;
pub mod connectivity;
pub mod dhcp;
pub mod dns;
pub mod stages;
pub mod vpn;
pub mod wifi;

use crate::config::{Config, OutputFormat};
use crate::render::color;

use super::{fail_icon, print_elevation_hint, success_icon};

pub struct StepResult {
    pub name: &'static str,
    pub success: bool,
    pub message: String,
}

pub fn print_step_ok(label: &str, config: &Config) {
    println!(
        "  {} {}",
        color::green(success_icon(config), config),
        color::green(label, config),
    );
}

pub fn print_step_fail(label: &str, detail: &str, config: &Config) {
    println!(
        "  {} {}",
        color::red(fail_icon(config), config),
        color::red(label, config),
    );
    if !detail.is_empty() {
        println!("    {}", color::dim(detail, config));
    }
}

pub fn warn_icon(config: &Config) -> &'static str {
    if config.use_unicode {
        crate::config::status_chars::WARN
    } else {
        crate::config::status_chars::WARN_ASCII
    }
}

pub async fn run(config: &Config) -> i32 {
    if config.format == OutputFormat::Json {
        return run_json(config).await;
    }

    println!();

    // Elevation check: all fix stages require root/admin on macOS/Linux
    if !crate::platform::is_elevated() {
        println!();
        println!(
            "  {}",
            color::red("The fix flow requires elevated privileges.", config),
        );
        print_elevation_hint(config);
        return 2;
    }

    // Step 0: Capture current SSID for potential Stage 3 reconnection
    let saved_ssid = wifi::capture_current_ssid().await;

    // Step 0b: VPN detection and disable
    let disabled_vpns = vpn::detect_and_disable(config).await;

    // ── Stage 1: Service Restart (automatic) ────────────────────────────────
    println!();
    println!(
        "  {} {}",
        color::cyan("Stage 1:", config),
        color::cyan("Service Restart", config),
    );

    let (stage1_steps, connected) = stages::run_stage1(config).await;

    if connected {
        println!();
        println!(
            "  {} {}",
            color::green(success_icon(config), config),
            color::green("Connectivity restored at Stage 1", config),
        );
        vpn::offer_reenable(&disabled_vpns, config).await;
        print_summary(config, 1, &stage1_steps, &[], &[]);
        return 0;
    }

    // ── Stage 2: Interface Reset (prompted) ─────────────────────────────────
    println!();
    println!(
        "  {} {}",
        color::cyan("Stage 2:", config),
        color::cyan("Interface Reset", config),
    );

    let (stage2_steps, connected) = stages::run_stage2(config).await;

    if connected {
        println!();
        println!(
            "  {} {}",
            color::green(success_icon(config), config),
            color::green("Connectivity restored at Stage 2", config),
        );
        vpn::offer_reenable(&disabled_vpns, config).await;
        print_summary(config, 2, &stage1_steps, &stage2_steps, &[]);
        return 0;
    }

    // ── Stage 3: Network Stack Reset (strong warning) ───────────────────────
    println!();
    println!(
        "  {} {}",
        color::cyan("Stage 3:", config),
        color::cyan("Network Stack Reset", config),
    );

    let (stage3_steps, connected) = stages::run_stage3(config, &saved_ssid).await;

    vpn::offer_reenable(&disabled_vpns, config).await;

    println!();
    if connected {
        println!(
            "  {} {}",
            color::green(success_icon(config), config),
            color::green("Connectivity restored at Stage 3", config),
        );
        print_summary(config, 3, &stage1_steps, &stage2_steps, &stage3_steps);
        0
    } else {
        println!(
            "  {} {}",
            color::red(fail_icon(config), config),
            color::red("Network fix completed but connectivity was not restored", config),
        );
        println!(
            "    {}",
            color::dim("The issue may require manual intervention or a system reboot.", config),
        );
        print_summary(config, 0, &stage1_steps, &stage2_steps, &stage3_steps);
        2
    }
}

fn print_summary(
    config: &Config,
    resolved_stage: u8,
    stage1: &[StepResult],
    stage2: &[StepResult],
    stage3: &[StepResult],
) {
    let all_steps: Vec<&StepResult> = stage1.iter().chain(stage2.iter()).chain(stage3.iter()).collect();
    let total = all_steps.len();
    let failed = all_steps.iter().filter(|s| !s.success).count();

    println!();
    if resolved_stage > 0 {
        println!(
            "  {} {}",
            color::green(success_icon(config), config),
            color::green(
                &format!("Network fix complete — resolved at stage {}", resolved_stage),
                config,
            ),
        );
    }
    if failed > 0 && resolved_stage == 0 {
        println!(
            "  {} {}",
            color::yellow(warn_icon(config), config),
            color::yellow(
                &format!("Fix completed with {} of {} steps failed", failed, total),
                config,
            ),
        );
    }
}

async fn run_json(config: &Config) -> i32 {
    // Elevation check: all fix stages require root/admin
    if !crate::platform::is_elevated() {
        let output = serde_json::json!({
            "action": "fix",
            "error": "elevated_privileges_required",
            "message": "Run with sudo (Unix) or as Administrator (Windows).",
            "stages": [],
            "resolved_at_stage": null,
            "requires_interaction": false,
            "vpn": null,
        });
        println!("{}", serde_json::to_string_pretty(&output).unwrap_or_else(|_| "{}".to_string()));
        return 2;
    }

    let saved_ssid = wifi::capture_current_ssid().await;
    let _ = saved_ssid; // Stage 3 is skipped in JSON mode

    // VPN: auto-disable consumer VPNs
    let disabled_vpns = vpn::detect_and_disable(config).await;

    // Stage 1
    let (stage1_steps, connected_s1) = stages::run_stage1(config).await;
    let stage1_json = steps_to_json(&stage1_steps);

    let mut stages = vec![serde_json::json!({
        "stage": 1,
        "name": "service_restart",
        "steps": stage1_json,
        "connectivity_restored": connected_s1,
    })];

    let mut resolved_at: Option<u8> = if connected_s1 { Some(1) } else { None };

    // Stage 2 (auto in JSON, no prompt)
    if resolved_at.is_none() {
        let (stage2_steps, connected_s2) = stages::run_stage2(config).await;
        let stage2_json = steps_to_json(&stage2_steps);
        stages.push(serde_json::json!({
            "stage": 2,
            "name": "interface_reset",
            "steps": stage2_json,
            "connectivity_restored": connected_s2,
        }));
        if connected_s2 {
            resolved_at = Some(2);
        }
    }

    // Stage 3 is SKIPPED in JSON mode
    let requires_interaction = resolved_at.is_none();

    let vpn_json = vpn::vpn_json(&disabled_vpns);

    let output = serde_json::json!({
        "action": "fix",
        "stages": stages,
        "resolved_at_stage": resolved_at,
        "requires_interaction": requires_interaction,
        "vpn": vpn_json,
    });

    println!("{}", serde_json::to_string_pretty(&output).unwrap_or_else(|_| "{}".to_string()));

    if resolved_at.is_some() { 0 } else { 2 }
}

fn steps_to_json(steps: &[StepResult]) -> Vec<serde_json::Value> {
    steps.iter().map(|s| {
        serde_json::json!({
            "name": s.name,
            "success": s.success,
            "message": s.message,
        })
    }).collect()
}
