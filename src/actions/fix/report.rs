use std::path::{Path, PathBuf};

use crate::config::Config;
use crate::render::color;

use super::StepResult;

/// Report data collected during the fix flow.
pub struct FixReport {
    pub timestamp: String,
    pub platform: String,
    pub resolved: bool,
    pub resolved_at_stage: Option<u8>,
    pub stages: Vec<StageReport>,
    pub vpn_names: Vec<String>,
    pub likely_cause: Option<String>,
}

pub struct StageReport {
    pub stage: u8,
    pub name: &'static str,
    pub steps: Vec<StepResult>,
    pub connectivity_restored: bool,
}

impl FixReport {
    pub fn new() -> Self {
        let timestamp = chrono::Local::now()
            .format("%Y-%m-%d %H:%M:%S")
            .to_string();
        Self {
            timestamp,
            platform: crate::platform::platform_name().to_string(),
            resolved: false,
            resolved_at_stage: None,
            stages: Vec::new(),
            vpn_names: Vec::new(),
            likely_cause: None,
        }
    }

    pub fn push_stage(
        &mut self,
        stage: u8,
        name: &'static str,
        steps: Vec<StepResult>,
        connectivity_restored: bool,
    ) {
        self.stages.push(StageReport {
            stage,
            name,
            steps,
            connectivity_restored,
        });
    }

    pub fn mark_resolved(&mut self, stage: u8) {
        self.resolved = true;
        self.resolved_at_stage = Some(stage);
    }

    pub fn finalize(&mut self) {
        self.likely_cause = infer_likely_cause(self);
    }
}

// ── Downloads directory resolution ──────────────────────────────────────────

fn downloads_dir() -> PathBuf {
    #[cfg(windows)]
    {
        if let Ok(profile) = std::env::var("USERPROFILE") {
            let dir = PathBuf::from(profile).join("Downloads");
            if dir.is_dir() {
                return dir;
            }
        }
    }

    #[cfg(not(windows))]
    {
        if let Ok(home) = std::env::var("HOME") {
            let dir = PathBuf::from(home).join("Downloads");
            if dir.is_dir() {
                return dir;
            }
        }
    }

    // Fallback: current working directory
    std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."))
}

// ── Root cause inference ────────────────────────────────────────────────────

fn infer_likely_cause(report: &FixReport) -> Option<String> {
    let resolved_stage = report.resolved_at_stage?;

    // Collect all step names from the resolved stage
    let stage_report = report.stages.iter().find(|s| s.stage == resolved_stage)?;
    let step_names: Vec<&str> = stage_report.steps.iter().map(|s| s.name).collect();
    let ok_names: Vec<&str> = stage_report
        .steps
        .iter()
        .filter(|s| s.success)
        .map(|s| s.name)
        .collect();

    match resolved_stage {
        1 => {
            if ok_names.contains(&"dns_reset_auto") {
                Some(
                    "DNS was misconfigured (not using DHCP-provided servers)".to_string(),
                )
            } else if step_names.contains(&"dns_set") {
                Some("DNS servers were unreachable or misconfigured".to_string())
            } else if ok_names.contains(&"service_restart")
                && !step_names.contains(&"dns_set")
            {
                Some(
                    "Network services needed restart (stale DHCP or DNS cache)".to_string(),
                )
            } else {
                Some("Network caches were stale and needed flushing".to_string())
            }
        }
        2 => Some(
            "Network interface was in a degraded state and needed to be reset".to_string(),
        ),
        3 => Some(
            "Network stack or connection profile was corrupted and required a full reset"
                .to_string(),
        ),
        _ => None,
    }
}

// ── Markdown report generation ──────────────────────────────────────────────

fn generate_markdown(report: &FixReport) -> String {
    let mut md = String::with_capacity(2048);

    md.push_str("# ND-300 Network Fix Report\n\n");
    md.push_str(&format!("**Date:** {}\n", report.timestamp));
    md.push_str(&format!("**Platform:** {}\n", report.platform));

    if let Some(stage) = report.resolved_at_stage {
        md.push_str(&format!("**Result:** Connectivity restored at Stage {}\n", stage));
    } else {
        md.push_str("**Result:** Not resolved\n");
    }

    // Summary
    md.push_str("\n## Summary\n\n");
    if let Some(stage) = report.resolved_at_stage {
        let stage_name = report
            .stages
            .iter()
            .find(|s| s.stage == stage)
            .map(|s| s.name)
            .unwrap_or("Unknown");
        md.push_str(&format!(
            "- **Resolved:** Yes, at Stage {} ({})\n",
            stage, stage_name
        ));
    } else {
        md.push_str("- **Resolved:** No\n");
    }
    if let Some(ref cause) = report.likely_cause {
        md.push_str(&format!("- **Likely cause:** {}\n", cause));
    } else {
        md.push_str("- **Likely cause:** Could not determine specific root cause\n");
    }

    // Stage details
    for stage_report in &report.stages {
        md.push_str(&format!(
            "\n## Stage {}: {}\n\n",
            stage_report.stage, stage_report.name
        ));

        if stage_report.steps.is_empty() {
            md.push_str("No steps were executed.\n");
            continue;
        }

        md.push_str("| Step | Result | Detail |\n");
        md.push_str("|------|--------|--------|\n");

        for step in &stage_report.steps {
            let result_str = if step.success { "OK" } else { "FAIL" };
            // Escape pipe characters in the message for Markdown table
            let detail = step.message.replace('|', "\\|");
            let name_display = step_display_name(step.name);
            md.push_str(&format!(
                "| {} | {} | {} |\n",
                name_display, result_str, detail
            ));
        }

        if stage_report.connectivity_restored {
            md.push_str("\n**Connectivity restored after this stage.**\n");
        }
    }

    // VPN section
    if !report.vpn_names.is_empty() {
        md.push_str("\n## VPN Activity\n\n");
        for name in &report.vpn_names {
            md.push_str(&format!("- Disabled: {} (re-enable offered after fix)\n", name));
        }
    }

    // Recommendations
    md.push_str("\n## Recommendations\n\n");
    if report.resolved {
        if let Some(ref cause) = report.likely_cause {
            if cause.contains("DNS") {
                md.push_str("- If the issue recurs, check your router's DNS settings or consider using a public DNS provider (Cloudflare 1.1.1.1 or Google 8.8.8.8)\n");
                md.push_str("- Ensure your DHCP server is providing valid DNS servers\n");
            } else if cause.contains("interface") {
                md.push_str("- If the issue recurs, check for driver updates for your network adapter\n");
                md.push_str("- Consider power cycling your router/modem\n");
            } else if cause.contains("stack") || cause.contains("corrupted") {
                md.push_str("- A system reboot is recommended to complete the reset\n");
                md.push_str("- If the issue recurs frequently, consider reinstalling network drivers\n");
            } else {
                md.push_str("- Monitor for recurrence and run `nd300 -t` for detailed diagnostics\n");
            }
        } else {
            md.push_str("- Monitor for recurrence and run `nd300 -t` for detailed diagnostics\n");
        }
    } else {
        md.push_str("- The automated fix could not restore connectivity\n");
        md.push_str("- Try rebooting your computer\n");
        md.push_str("- Check physical connections (Ethernet cable, Wi-Fi router power)\n");
        md.push_str("- Contact your ISP if the issue persists after reboot\n");
        md.push_str("- Run `nd300 -t` for detailed diagnostics to share with support\n");
    }

    // Footer
    md.push_str(&format!(
        "\n---\n*Generated by ND-300 v{}*\n",
        env!("CARGO_PKG_VERSION")
    ));

    md
}

/// Convert internal step names to human-readable labels.
fn step_display_name(name: &str) -> &str {
    match name {
        "dns_reset_auto" => "DNS reset to automatic (DHCP)",
        "dns_flush" => "DNS cache flushed",
        "arp_flush" => "ARP cache flushed",
        "service_restart" => "Network services restarted",
        "dhcp_renew" => "DHCP lease renewed",
        "dns_set" => "DNS servers configured",
        "interface_reset" => "Interface reset",
        "stack_reset" => "Network stack reset",
        _ => name,
    }
}

// ── File saving ─────────────────────────────────────────────────────────────

/// Save the Markdown report to the Downloads directory.
/// Returns the file path on success, or `None` on any error.
pub fn save_report(report: &FixReport) -> Option<PathBuf> {
    let dir = downloads_dir();

    // Ensure directory exists (best-effort)
    let _ = std::fs::create_dir_all(&dir);

    // Generate a timestamp-based filename
    let filename_ts = chrono::Local::now().format("%Y%m%d-%H%M%S").to_string();
    let filename = format!("nd300-fix-report-{}.md", filename_ts);
    let path = dir.join(filename);

    let markdown = generate_markdown(report);

    match std::fs::write(&path, markdown) {
        Ok(()) => Some(path),
        Err(_) => None,
    }
}

// ── Terminal summary ────────────────────────────────────────────────────────

/// Print a concise summary box after the fix flow completes.
pub fn print_terminal_summary(
    report: &FixReport,
    saved_path: Option<&Path>,
    config: &Config,
) {
    let h = if config.use_unicode { '─' } else { '-' };
    let rule: String = std::iter::repeat(h).take(50).collect();

    println!();
    println!(
        "  {} {}",
        color::bold("Fix Summary", config),
        color::dim(&rule, config),
    );

    // Result line
    let result_text = if let Some(stage) = report.resolved_at_stage {
        let stage_name = report
            .stages
            .iter()
            .find(|s| s.stage == stage)
            .map(|s| s.name)
            .unwrap_or("Unknown");
        format!("Connectivity restored at Stage {} ({})", stage, stage_name)
    } else {
        "Not resolved — manual intervention may be needed".to_string()
    };

    if report.resolved {
        println!(
            "  {:<14}{}",
            "Result:",
            color::green(&result_text, config),
        );
    } else {
        println!(
            "  {:<14}{}",
            "Result:",
            color::red(&result_text, config),
        );
    }

    // Likely cause
    if let Some(ref cause) = report.likely_cause {
        println!(
            "  {:<14}{}",
            "Likely cause:",
            color::cyan(cause, config),
        );
    } else {
        println!(
            "  {:<14}{}",
            "Likely cause:",
            color::dim("Could not determine specific root cause", config),
        );
    }

    // Changes applied (list of successful steps)
    let changes: Vec<&str> = report
        .stages
        .iter()
        .flat_map(|s| s.steps.iter())
        .filter(|s| s.success)
        .map(|s| step_display_name(s.name))
        .collect();

    if !changes.is_empty() {
        let changes_str = changes.join(", ");
        // Wrap long lines
        if changes_str.len() > 60 {
            println!("  {:<14}{}", "Changes:", &changes_str[..60]);
            let mut remaining = &changes_str[60..];
            while !remaining.is_empty() {
                let end = remaining.len().min(60);
                println!("  {:<14}{}", "", &remaining[..end]);
                remaining = &remaining[end..];
            }
        } else {
            println!("  {:<14}{}", "Changes:", changes_str);
        }
    }

    // VPNs
    if !report.vpn_names.is_empty() {
        let vpn_str = report.vpn_names.join(", ");
        println!(
            "  {:<14}{}",
            "VPNs disabled:",
            color::yellow(&vpn_str, config),
        );
    }

    // Report path
    match saved_path {
        Some(path) => {
            println!(
                "  {:<14}{}",
                "Report saved:",
                color::dim(&path.display().to_string(), config),
            );
        }
        None => {
            println!(
                "  {:<14}{}",
                "Report:",
                color::dim("Could not save report file", config),
            );
        }
    }
}
