//! Diagnostic-driven fix loop driver.
//!
//! The flow:
//!
//! 1. Run baseline diagnostics.
//! 2. If everything passes, exit cleanly.
//! 3. Otherwise, in a bounded loop:
//!    a. Detect hard blocks (captive portal / ISP outage / no link / enterprise VPN) — exit cleanly with guidance.
//!    b. Compute the actionable failure set, group by root cause, and build a plan.
//!    c. Apply the plan's actions one by one, prompting Y/N for any High-risk action.
//!    d. After each action, sleep its `stabilization` window.
//!    e. Re-run diagnostics; if all pass, exit; else continue.
//! 4. Bounded by iteration count, wall clock, and per-action attempt caps.

use std::time::{Duration, Instant};

use crate::config::{Config, OutputFormat};
use crate::diagnostics;

use super::action::{self, DiagnosticKey};
use super::session::{FinalOutcome, Reporter, Session, DEFAULT_ITERATION_DELAY};
use super::triage::{
    actionable_failures, build_plan, hard_block_detected, requires_confirmation,
    requires_high_risk_consent, HardBlock, MAX_ITERATIONS,
};

/// Top-level entry point. Runs the full triage loop and returns a process
/// exit code. Always produces a `Session` populated with the timeline; the
/// caller persists the Markdown report.
pub async fn run(config: &Config) -> (Session, FinalOutcome) {
    let interactive = is_interactive(config);
    let reporter = Reporter::new(config);
    let mut session = Session::new();

    if interactive {
        reporter.header();
    }

    // Iteration 1: baseline diagnostics.
    let baseline = diagnostics::run_all(config).await;
    session.record_baseline(baseline.clone());

    let mut current = baseline;

    if interactive {
        let initial_failures = actionable_failures(&current);
        reporter.baseline_summary(initial_failures.len());
    }

    for iteration in 1..=MAX_ITERATIONS {
        // Wall-clock cap.
        if session.wall_clock_exhausted() {
            let remaining: Vec<DiagnosticKey> = actionable_failures(&current).into_iter().collect();
            let outcome = FinalOutcome::Timeout(remaining);
            session.final_outcome = Some(outcome.clone());
            if interactive {
                reporter.final_verdict(&outcome, None);
            }
            return (session, outcome);
        }

        let failures = actionable_failures(&current);
        if failures.is_empty() {
            let outcome = FinalOutcome::Fixed;
            session.final_outcome = Some(outcome.clone());
            if interactive {
                reporter.final_verdict(&outcome, None);
            }
            return (session, outcome);
        }

        // Hard-block check — short-circuits before any action runs.
        if let Some(block) = hard_block_detected(&current) {
            let outcome = FinalOutcome::HardBlock(block);
            session.final_outcome = Some(outcome.clone());
            if interactive {
                reporter.final_verdict(&outcome, None);
            }
            return (session, outcome);
        }

        if interactive {
            reporter.iteration_header(iteration);
        }

        let registry = action::all_actions();
        let plan = build_plan(
            &failures,
            &session.attempts,
            &session.effectiveness,
            &registry,
        );

        if plan.is_empty() {
            let remaining: Vec<DiagnosticKey> = failures.into_iter().collect();
            let outcome = FinalOutcome::Exhausted(remaining);
            session.final_outcome = Some(outcome.clone());
            if interactive {
                reporter.final_verdict(&outcome, None);
            }
            return (session, outcome);
        }

        // Apply actions in cost-order. Fatal env changes break early so we
        // re-probe before applying further actions in the same iteration.
        let mut user_declined_confirmation = false;
        let mut skipped_for_confirmation = false;
        let mut ran_action = false;
        for action in &plan {
            if session.wall_clock_exhausted() {
                break;
            }

            // Confirmation gates. High-risk always requires explicit Y/N;
            // medium-risk and DNS-changing actions honor --yes.
            if requires_confirmation(action, config.auto_confirm_medium_risk) {
                if !interactive {
                    session.record_action(
                        iteration,
                        action,
                        super::action::ActionOutcome::fail(
                            "Skipped: requires confirmation. Re-run `nd300 fix` in a terminal or use `--yes` for medium-risk actions.",
                        ),
                        Duration::from_millis(0),
                        false,
                        true,
                    );
                    skipped_for_confirmation = true;
                    continue;
                }

                let approved = if requires_high_risk_consent(action) {
                    reporter.high_risk_prompt(action)
                } else {
                    reporter.confirmation_prompt(action)
                };

                if !approved {
                    reporter.confirmation_declined(action);
                    session.record_action(
                        iteration,
                        action,
                        super::action::ActionOutcome::fail("User declined the prompt."),
                        Duration::from_millis(0),
                        true,
                        false,
                    );
                    user_declined_confirmation = true;
                    break;
                }
            }

            if interactive {
                reporter.announce_action(action);
            }
            let started = Instant::now();
            let outcome = action.apply(config).await;
            let duration = started.elapsed();
            if interactive {
                reporter.finish_action(&outcome, duration);
            }

            let fatal_env_change = outcome.fatal_environment_change;
            session.record_action(iteration, action, outcome, duration, false, false);
            ran_action = true;

            // Stabilize before either re-probing or applying the next action.
            if action.stabilization > Duration::from_millis(0) {
                tokio::time::sleep(action.stabilization).await;
            }

            if fatal_env_change {
                // Break out of the plan-loop and re-probe immediately.
                break;
            }
        }

        if user_declined_confirmation || (skipped_for_confirmation && !ran_action) {
            let remaining: Vec<DiagnosticKey> = actionable_failures(&current).into_iter().collect();
            let outcome = FinalOutcome::UserDeclined(remaining);
            session.final_outcome = Some(outcome.clone());
            if interactive {
                reporter.final_verdict(&outcome, None);
            }
            return (session, outcome);
        }

        // Light delay between iterations to let the OS settle.
        tokio::time::sleep(DEFAULT_ITERATION_DELAY).await;

        // Re-probe.
        let prior_failures = actionable_failures(&current);
        current = diagnostics::run_all(config).await;
        let now_failures = actionable_failures(&current);
        session.record_iteration(iteration, current.clone());
        session.update_effectiveness(iteration, &prior_failures, &now_failures);
    }

    // Hit MAX_ITERATIONS without converging.
    let remaining_failures = actionable_failures(&current);
    let remaining: Vec<DiagnosticKey> = remaining_failures.iter().copied().collect();
    let outcome = if remaining_failures.is_empty() {
        FinalOutcome::Fixed
    } else {
        let baseline_failures = session
            .baseline
            .as_ref()
            .map(actionable_failures)
            .unwrap_or_default();
        let any_progress = baseline_failures
            .difference(&remaining_failures)
            .next()
            .is_some();
        if any_progress {
            FinalOutcome::Partial(remaining)
        } else {
            FinalOutcome::Exhausted(remaining)
        }
    };
    session.final_outcome = Some(outcome.clone());
    if interactive {
        reporter.final_verdict(&outcome, None);
    }
    (session, outcome)
}

/// True when the loop can render interactive prompts (TTY + non-JSON output).
fn is_interactive(config: &Config) -> bool {
    use std::io::IsTerminal;
    config.format != OutputFormat::Json && std::io::stdin().is_terminal()
}

/// Convenience wrapper used by `actions::fix::run`. Persists the Markdown
/// report and returns the exit code derived from the `FinalOutcome`.
pub async fn run_and_finalize(config: &Config) -> i32 {
    // Pre-flight: elevation
    if !crate::platform::is_elevated() {
        let outcome = FinalOutcome::PreflightFailed(
            "The fix flow requires elevated privileges. Run with sudo (Unix) or as Administrator (Windows).".to_string(),
        );
        if config.format == OutputFormat::Json {
            print_json_outcome(&Session::new(), &outcome, None);
        } else {
            let reporter = Reporter::new(config);
            reporter.final_verdict(&outcome, None);
        }
        return outcome.exit_code();
    }

    let (session, outcome) = run(config).await;
    let report_path = super::report::save_session_report(&session, &outcome);

    if config.format == OutputFormat::Json {
        print_json_outcome(&session, &outcome, report_path.as_deref());
    } else if let Some(path) = &report_path {
        // Re-print the path under the verdict so users see where to find it.
        // (final_verdict was already called inside run(); this just adds a follow-up.)
        println!(
            "  {} {}",
            crate::render::color::dim("Saved report:", config),
            crate::render::color::dim(&path.display().to_string(), config),
        );
    }

    outcome.exit_code()
}

fn print_json_outcome(
    session: &Session,
    outcome: &FinalOutcome,
    report_path: Option<&std::path::Path>,
) {
    use serde_json::json;

    let outcome_label = match outcome {
        FinalOutcome::Fixed => "fixed",
        FinalOutcome::Partial(_) => "partial",
        FinalOutcome::Exhausted(_) => "exhausted",
        FinalOutcome::HardBlock(_) => "hard_block",
        FinalOutcome::Timeout(_) => "timeout",
        FinalOutcome::UserDeclined(_) => "user_declined",
        FinalOutcome::PreflightFailed(_) => "preflight_failed",
    };

    let remaining: Vec<&str> = match outcome {
        FinalOutcome::Partial(rs)
        | FinalOutcome::Exhausted(rs)
        | FinalOutcome::Timeout(rs)
        | FinalOutcome::UserDeclined(rs) => rs.iter().map(|k| diagnostic_key_str(*k)).collect(),
        _ => Vec::new(),
    };

    let actions_json: Vec<_> = session
        .action_log
        .iter()
        .map(|r| {
            json!({
                "iteration": r.iteration,
                "action": format!("{:?}", r.action_id),
                "label": r.label,
                "ok": r.outcome.ok,
                "message": r.outcome.message,
                "duration_ms": r.duration.as_millis() as u64,
                "user_declined": r.user_declined,
                "skipped_no_interaction": r.skipped_no_interaction,
            })
        })
        .collect();

    let value = json!({
        "action": "fix",
        "outcome": outcome_label,
        "exit_code": outcome.exit_code(),
        "iterations": session.snapshots.len().saturating_sub(1),
        "remaining_failures": remaining,
        "applied_actions": actions_json,
        "elapsed_seconds": session.elapsed().as_secs(),
        "report_path": report_path.map(|p| p.display().to_string()),
        "preflight_error": match outcome {
            FinalOutcome::PreflightFailed(s) => Some(s.clone()),
            _ => None,
        },
        "hard_block": match outcome {
            FinalOutcome::HardBlock(b) => Some(hard_block_str(b).to_string()),
            _ => None,
        },
    });

    println!(
        "{}",
        serde_json::to_string_pretty(&value).unwrap_or_else(|_| "{}".to_string())
    );
}

fn diagnostic_key_str(k: DiagnosticKey) -> &'static str {
    match k {
        DiagnosticKey::Adapters => "adapters",
        DiagnosticKey::Interfaces => "interfaces",
        DiagnosticKey::Gateway => "gateway",
        DiagnosticKey::Dns => "dns",
        DiagnosticKey::PublicIp => "public_ip",
        DiagnosticKey::Latency => "latency",
        DiagnosticKey::Ports => "ports",
        DiagnosticKey::Speed => "speed",
    }
}

fn hard_block_str(b: &HardBlock) -> &'static str {
    match b {
        HardBlock::CaptivePortal => "captive_portal",
        HardBlock::NoPhysicalLink => "no_physical_link",
        HardBlock::IspOutage => "isp_outage",
        HardBlock::EnterpriseVpnActive(_) => "enterprise_vpn_active",
    }
}
