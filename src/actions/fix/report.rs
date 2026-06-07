//! Markdown report generation for the diagnostic-driven fix loop.
//!
//! Writes `~/Downloads/nd300-fix-report-<timestamp>.md` containing:
//!
//! 1. Plain summary (matches the terminal verdict).
//! 2. Baseline diagnostics snapshot.
//! 3. Iteration timeline — for each pass: failures, actions applied, captured
//!    stdout/stderr/duration, and the diagnostic delta after.
//! 4. Final diagnostics snapshot.
//! 5. Environment block.
//! 6. "What to try next" suggestions (only when not fully fixed).

use std::path::PathBuf;

use crate::diagnostics::{DiagnosticResult, DiagnosticResults, DiagnosticStatus};

use super::action::DiagnosticKey;
use super::session::{suggestions_for_keys, FinalOutcome, Session};

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

    std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."))
}

/// Save a Markdown report for the session to ~/Downloads/. Returns the path
/// on success, or `None` on any IO failure (the loop is best-effort about
/// reports — a write failure should never block exit).
pub fn save_session_report(session: &Session, outcome: &FinalOutcome) -> Option<PathBuf> {
    save_session_report_with_recovery(session, outcome, &[])
}

/// Like [`save_session_report`], but also records any manual-recovery items the
/// restore drain could not handle (a still-disabled adapter, a VPN that
/// wouldn't reconnect, a macOS service that couldn't be recreated).
pub fn save_session_report_with_recovery(
    session: &Session,
    outcome: &FinalOutcome,
    recovery_needed: &[String],
) -> Option<PathBuf> {
    let dir = downloads_dir();
    let _ = std::fs::create_dir_all(&dir);

    let stamp = chrono::Local::now().format("%Y%m%d-%H%M%S").to_string();
    let path = dir.join(format!("nd300-fix-report-{}.md", stamp));
    let markdown = render_markdown(session, outcome, recovery_needed);

    match std::fs::write(&path, markdown) {
        Ok(()) => Some(path),
        Err(_) => None,
    }
}

// ── Markdown renderer ────────────────────────────────────────────────────────

fn render_markdown(
    session: &Session,
    outcome: &FinalOutcome,
    recovery_needed: &[String],
) -> String {
    let mut md = String::with_capacity(8 * 1024);

    let stamp = chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string();

    md.push_str("# ND-300 Fix Report\n\n");
    md.push_str(&format!("**Generated:** {}\n", stamp));
    md.push_str(&format!(
        "**Platform:** {}\n",
        crate::platform::platform_name()
    ));
    md.push_str(&format!(
        "**ND-300 version:** {}\n",
        env!("CARGO_PKG_VERSION")
    ));
    md.push_str(&format!(
        "**Wall-clock duration:** {} seconds\n",
        session.elapsed().as_secs()
    ));
    md.push('\n');

    // Summary verdict
    md.push_str("## Verdict\n\n");
    md.push_str(&render_verdict(outcome));
    md.push('\n');

    // Manual recovery needed (drain failures) — surfaced prominently right
    // under the verdict so a stranded user finds it first.
    if !recovery_needed.is_empty() {
        md.push_str("## ⚠️ Manual recovery needed\n\n");
        md.push_str(
            "nd300 tried to restore the network state it changed, but the following could not be \
             completed automatically. Please address them manually:\n\n",
        );
        for item in recovery_needed {
            md.push_str(&format!("- {}\n", item));
        }
        md.push('\n');
    }

    // Baseline
    if let Some(baseline) = &session.baseline {
        md.push_str("## Baseline diagnostics (before any actions)\n\n");
        md.push_str(&render_diagnostic_table(baseline));
        md.push('\n');
    }

    // Iteration timeline
    md.push_str("## Iteration timeline\n\n");
    if session.action_log.is_empty() {
        md.push_str("_No actions were applied — there was nothing to fix._\n\n");
    } else {
        let max_iter = session
            .action_log
            .iter()
            .map(|r| r.iteration)
            .max()
            .unwrap_or(0);
        for iter in 1..=max_iter {
            md.push_str(&format!("### Iteration {}\n\n", iter));
            let actions_in_iter: Vec<_> = session
                .action_log
                .iter()
                .filter(|r| r.iteration == iter)
                .collect();
            if actions_in_iter.is_empty() {
                md.push_str("_No actions applied this iteration._\n\n");
                continue;
            }
            for record in actions_in_iter {
                let status_icon = if record.outcome.ok {
                    "✅"
                } else if record.skipped_no_interaction {
                    "⏭️"
                } else if record.user_declined {
                    "🚫"
                } else {
                    "❌"
                };
                md.push_str(&format!(
                    "**{}** `{:?}` — {}\n\n",
                    status_icon, record.action_id, record.label
                ));
                md.push_str(&format!(
                    "- **Result:** {} ({:.1}s)\n",
                    record.outcome.message,
                    record.duration.as_secs_f64()
                ));
                if !record.outcome.cmd_outcomes.is_empty() {
                    md.push_str("- **Commands run:**\n\n");
                    for cmd in &record.outcome.cmd_outcomes {
                        md.push_str("  ```\n");
                        md.push_str(&format!("  $ {}\n", cmd.cmdline()));
                        if !cmd.stdout.trim().is_empty() {
                            md.push_str("  --- stdout ---\n");
                            for line in cmd.stdout.lines().take(20) {
                                md.push_str(&format!("  {}\n", line));
                            }
                        }
                        if !cmd.stderr.trim().is_empty() {
                            md.push_str("  --- stderr ---\n");
                            for line in cmd.stderr.lines().take(20) {
                                md.push_str(&format!("  {}\n", line));
                            }
                        }
                        if let Some(code) = cmd.exit_code {
                            md.push_str(&format!(
                                "  exit {} ({:.1}s)\n",
                                code,
                                cmd.duration.as_secs_f64()
                            ));
                        }
                        md.push_str("  ```\n\n");
                    }
                }
                md.push('\n');
            }
        }
    }

    // Final snapshot
    if let Some(last) = session.snapshots.last() {
        md.push_str("## Final diagnostics (after all actions)\n\n");
        md.push_str(&render_diagnostic_table(&last.results));
        md.push('\n');
    }

    // Suggestions
    let remaining_keys = remaining_failures(outcome);
    if !remaining_keys.is_empty() {
        md.push_str("## What to try next\n\n");
        for s in suggestions_for_keys(&remaining_keys) {
            md.push_str(&format!("- {}\n", s));
        }
        md.push('\n');
    }

    // Environment
    md.push_str("## Environment\n\n");
    md.push_str(&format!(
        "- Platform: {}\n",
        crate::platform::platform_name()
    ));
    md.push_str(&format!("- Elevated: {}\n", crate::platform::is_elevated()));
    md.push_str(&format!(
        "- ND-300 version: {}\n",
        env!("CARGO_PKG_VERSION")
    ));
    if !session.vpn_names.is_empty() {
        md.push_str(&format!(
            "- VPNs detected: {}\n",
            session.vpn_names.join(", ")
        ));
    }
    md.push('\n');

    md.push_str("---\n");
    md.push_str("_This report was generated automatically by `nd300 fix`._\n");

    md
}

fn render_verdict(outcome: &FinalOutcome) -> String {
    match outcome {
        FinalOutcome::Fixed => {
            "✅ **Fixed** — Connectivity is healthy. The actions above resolved the failures.\n"
                .to_string()
        }
        FinalOutcome::Partial(remaining) => {
            format!(
                "⚠️ **Partially fixed** — Some failures cleared, but the following remain: {}\n",
                describe_keys(remaining)
            )
        }
        FinalOutcome::Exhausted(remaining) => {
            format!(
                "❌ **Couldn't fix** — Every applicable action was attempted. Still failing: {}\n",
                describe_keys(remaining)
            )
        }
        FinalOutcome::HardBlock(reason) => {
            format!("⚠️ **Cannot fix from here.** {}\n", reason.user_message())
        }
        FinalOutcome::Timeout(remaining) => {
            format!(
                "❌ **Timed out** — Loop hit its safety cap. Still failing: {}\n",
                describe_keys(remaining)
            )
        }
        FinalOutcome::UserDeclined(remaining) => {
            format!(
                "⚠️ **Stopped at your request** — A high-risk action was declined. Still failing: {}\n",
                describe_keys(remaining)
            )
        }
        FinalOutcome::PreflightFailed(reason) => {
            format!("❌ **Could not start.** {}\n", reason)
        }
        FinalOutcome::Interrupted(remaining) => {
            if remaining.is_empty() {
                "⚠️ **Interrupted** — The fix was stopped before it finished. nd300 attempted to restore any network state it had changed.\n".to_string()
            } else {
                format!(
                    "⚠️ **Interrupted** — The fix was stopped before it finished. nd300 attempted to restore any network state it had changed. Still outstanding when interrupted: {}\n",
                    describe_keys(remaining)
                )
            }
        }
    }
}

fn remaining_failures(outcome: &FinalOutcome) -> Vec<DiagnosticKey> {
    match outcome {
        FinalOutcome::Partial(rs)
        | FinalOutcome::Exhausted(rs)
        | FinalOutcome::Timeout(rs)
        | FinalOutcome::UserDeclined(rs)
        | FinalOutcome::Interrupted(rs) => rs.clone(),
        _ => Vec::new(),
    }
}

fn describe_keys(keys: &[DiagnosticKey]) -> String {
    let names: Vec<&str> = keys.iter().map(|k| key_label(*k)).collect();
    names.join(", ")
}

fn key_label(k: DiagnosticKey) -> &'static str {
    match k {
        DiagnosticKey::Adapters => "network adapter",
        DiagnosticKey::Interfaces => "network interface",
        DiagnosticKey::Gateway => "gateway / router",
        DiagnosticKey::Dns => "DNS",
        DiagnosticKey::PublicIp => "public IP",
        DiagnosticKey::Latency => "latency",
        DiagnosticKey::Ports => "outbound ports",
        DiagnosticKey::Speed => "speed test",
    }
}

fn render_diagnostic_table(r: &DiagnosticResults) -> String {
    let rows: Vec<(&str, &DiagnosticResult)> = vec![
        ("Adapters", &r.adapters),
        ("Interfaces", &r.interfaces),
        ("Gateway", &r.gateway),
        ("DNS", &r.dns),
        ("Public IP", &r.public_ip),
        ("Latency", &r.latency),
        ("Ports", &r.ports),
        ("Speed", &r.speed),
    ];

    let mut s = String::new();
    s.push_str("| Diagnostic | Status | Summary |\n");
    s.push_str("|------------|--------|---------|\n");
    for (label, res) in rows {
        let status = match res.status {
            DiagnosticStatus::Ok => "✅ OK",
            DiagnosticStatus::Warn => "⚠️ Warn",
            DiagnosticStatus::Fail => "❌ Fail",
            DiagnosticStatus::Skip => "— Skip",
        };
        let summary = res.summary.replace('|', "\\|");
        s.push_str(&format!("| {} | {} | {} |\n", label, status, summary));
    }
    s
}
