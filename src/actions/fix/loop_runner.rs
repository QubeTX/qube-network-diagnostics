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
use super::session::{FinalOutcome, Reporter, RestoreRegistry, Session, DEFAULT_ITERATION_DELAY};
use super::triage::{
    actionable_failures, build_plan, confirmed_failures, hard_block_detected,
    intermittent_failures, requires_confirmation, requires_high_risk_consent, HardBlock,
    MAX_ITERATIONS,
};

/// Wait for any supported operator/process-manager interruption. Unix service
/// managers commonly use SIGTERM, and terminal/session loss can deliver
/// SIGHUP; both must take the same restore-drain path as Ctrl-C.
async fn fix_interrupt_signal() {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{signal, SignalKind};

        let term = signal(SignalKind::terminate());
        let hup = signal(SignalKind::hangup());
        match (term, hup) {
            (Ok(mut term), Ok(mut hup)) => {
                tokio::select! {
                    _ = tokio::signal::ctrl_c() => {},
                    _ = term.recv() => {},
                    _ = hup.recv() => {},
                }
            }
            _ => {
                let _ = tokio::signal::ctrl_c().await;
            }
        }
    }

    #[cfg(not(unix))]
    {
        let _ = tokio::signal::ctrl_c().await;
    }
}

/// Seam for injecting scripted diagnostics into the triage loop in tests.
/// The real implementation wraps `diagnostics::run_all` with the loop's
/// speed-skipping config.
pub(crate) trait DiagProbe {
    async fn probe(&mut self) -> diagnostics::DiagnosticResults;
}

struct RealProbe {
    config: Config,
}

impl DiagProbe for RealProbe {
    async fn probe(&mut self) -> diagnostics::DiagnosticResults {
        diagnostics::run_all(&self.config, diagnostics::run_all_cap(&self.config)).await
    }
}

/// Runs the full triage loop, populating the caller-owned `session` so the
/// report stays rich even if the run is interrupted or panics. Returns the
/// `FinalOutcome`. Destructive actions register inverse ops on `restore`; the
/// caller drains it on every terminal path.
pub async fn run(
    config: &Config,
    session: &mut Session,
    restore: &RestoreRegistry,
) -> FinalOutcome {
    // Diagnostics inside the fix loop never run the speed test: no action
    // targets Speed (pinned by `triage::tests::no_action_targets_speed`), and
    // a ~40s+ sequential speed test per pass would spend the 240s wall-clock
    // budget on re-probing instead of repairing.
    let mut probe = RealProbe {
        config: config.clone().with_skip_speed(),
    };
    run_with_probe(&mut probe, config, session, restore).await
}

async fn run_with_probe(
    probe: &mut impl DiagProbe,
    config: &Config,
    session: &mut Session,
    restore: &RestoreRegistry,
) -> FinalOutcome {
    let interactive = is_interactive(config);
    let reporter = Reporter::new(config);

    if interactive {
        reporter.header();
    }

    // Iteration 1: baseline diagnostics.
    let baseline = probe.probe().await;
    session.record_baseline(baseline.clone());

    let first_failures = actionable_failures(&baseline);

    if interactive {
        reporter.baseline_summary(first_failures.len());
    }

    let mut current = baseline;

    // Evidence gate: a failing baseline is re-confirmed with a second pass
    // before the first repair plan. Only failures present in BOTH passes are
    // actionable in iteration 1 — a transient blip self-clears here instead
    // of triggering a repair. Failures that flicker between the passes are
    // recorded as intermittent so their later natural recoveries earn no
    // effectiveness credit.
    let mut confirmed_for_iter1: Option<std::collections::HashSet<DiagnosticKey>> = None;
    if !first_failures.is_empty() {
        if interactive {
            reporter.confirmation_pass();
        }
        let second = probe.probe().await;
        let second_failures = actionable_failures(&second);
        let confirmed = confirmed_failures(&first_failures, &second_failures);
        let intermittent = intermittent_failures(&first_failures, &second_failures);
        if interactive {
            reporter.confirmation_result(confirmed.len(), intermittent.len());
        }
        session.record_confirmation(second.clone(), intermittent);
        // The freshest snapshot drives the loop (hard-block detection
        // included), so a persistent hard-block shape still short-circuits
        // and a transient one self-clears.
        current = second;
        confirmed_for_iter1 = Some(confirmed);
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
            return outcome;
        }

        // Iteration 1 plans against the confirmed evidence set; later
        // iterations use their single re-probe as today (every failure they
        // see has already been observed in at least two runs).
        let failures = confirmed_for_iter1
            .take()
            .unwrap_or_else(|| actionable_failures(&current));
        if failures.is_empty() {
            let outcome = FinalOutcome::Fixed;
            session.final_outcome = Some(outcome.clone());
            if interactive {
                reporter.final_verdict(&outcome, None);
            }
            return outcome;
        }

        // Hard-block check — short-circuits before any action runs.
        if let Some(block) = hard_block_detected(&current) {
            let outcome = FinalOutcome::HardBlock(block);
            session.final_outcome = Some(outcome.clone());
            if interactive {
                reporter.final_verdict(&outcome, None);
            }
            return outcome;
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
            return outcome;
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
            let outcome = action.apply(config, restore).await;
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
            return outcome;
        }

        // Light delay between iterations to let the OS settle.
        tokio::time::sleep(DEFAULT_ITERATION_DELAY).await;

        // Re-probe.
        let prior_failures = actionable_failures(&current);
        current = probe.probe().await;
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
    outcome
}

/// True when the loop can render interactive prompts (TTY + non-JSON output).
fn is_interactive(config: &Config) -> bool {
    use std::io::IsTerminal;
    config.format != OutputFormat::Json && std::io::stdin().is_terminal()
}

/// Convenience wrapper used by `actions::fix::run`. Persists the Markdown
/// report and returns the exit code derived from the `FinalOutcome`.
///
/// This is the interrupt-safe boundary: the triage loop runs inside a
/// `tokio::select!` that races it against Ctrl-C/SIGTERM/SIGHUP, and the loop future is
/// wrapped in `catch_unwind` so a panic is caught rather than aborting the
/// process. On EVERY terminal path — normal end, user-declined, wall-clock cap,
/// Ctrl-C, or panic — the restore registry is drained so any half-applied
/// network change (a disabled adapter, a disconnected VPN, or a temporarily
/// disabled macOS service) is rolled back before the process exits.
pub async fn run_and_finalize(config: &Config) -> i32 {
    use futures_util::FutureExt;

    // Pre-flight: elevation
    if !crate::platform::is_elevated() {
        let outcome = FinalOutcome::PreflightFailed(
            "The fix flow requires elevated privileges. Run with sudo (Unix) or as Administrator (Windows).".to_string(),
        );
        if config.format == OutputFormat::Json {
            print_json_outcome(&Session::new(), &outcome, None, &[]);
        } else {
            let reporter = Reporter::new(config);
            reporter.final_verdict(&outcome, None);
        }
        return outcome.exit_code();
    }

    let is_json = config.format == OutputFormat::Json;
    let mut session = Session::new();
    let restore = RestoreRegistry::new();

    // Race the loop against Ctrl-C/SIGTERM/SIGHUP, and catch any panic from the
    // loop so we can still drain restores instead of leaving the network
    // half-broken.
    //
    // `AssertUnwindSafe` is sound here: the registry uses a non-poisoning
    // `tokio::sync::Mutex`, and after a caught panic we only READ the partially
    // populated `Session` to build a best-effort report — we never rely on it
    // being in a logically-consistent state.
    let loop_result = {
        let fut = std::panic::AssertUnwindSafe(run(config, &mut session, &restore)).catch_unwind();
        tokio::select! {
            biased;
            _ = fix_interrupt_signal() => None,
            r = fut => Some(r),
        }
    };

    // Classify the terminal path.
    //   None                -> a supported signal interrupted the loop.
    //   Some(Ok(outcome))   -> loop finished normally (verdict already printed).
    //   Some(Err(_panic))   -> loop panicked (caught); re-raise after cleanup.
    let (outcome, panicked) = match loop_result {
        Some(Ok(outcome)) => (outcome, false),
        Some(Err(_panic)) => (
            FinalOutcome::Interrupted(remaining_after_interrupt(&session)),
            true,
        ),
        None => {
            // Signal: print a clear interrupted line now (the loop never
            // returned, so it never printed a verdict).
            if !is_json {
                println!();
                println!("  Interrupted — cleaning up and restoring network state...");
            }
            (
                FinalOutcome::Interrupted(remaining_after_interrupt(&session)),
                false,
            )
        }
    };

    if panicked && !is_json {
        println!();
        println!(
            "  A fatal internal error occurred mid-fix — restoring network state before exiting..."
        );
    }

    // ALWAYS drain restores, regardless of how we got here. The aggregate
    // bound scales with the pending snapshot so every LIFO entry receives its
    // own bounded attempt; a fixed 90-second cap could skip entry four onward.
    let drain_timeout = restore.required_drain_timeout().await;
    let drain_failures = match tokio::time::timeout(drain_timeout, restore.drain()).await {
        Ok(failures) => failures,
        Err(_) => vec![format!(
            "Network-state cleanup did not finish within {}s; some changes may not have been restored.",
            drain_timeout.as_secs()
        )],
    };

    // For the Interrupted path, print the verdict now (after the drain attempt)
    // so the manual-recovery guidance reads in order.
    if matches!(outcome, FinalOutcome::Interrupted(_)) && !is_json {
        let reporter = Reporter::new(config);
        reporter.final_verdict(&outcome, None);
    }

    // Surface anything that couldn't be restored as explicit manual-recovery
    // guidance (non-JSON; JSON carries it in the structured object).
    if !drain_failures.is_empty() && !is_json {
        println!();
        println!(
            "  {}",
            crate::render::color::yellow("Manual recovery needed:", config)
        );
        for f in &drain_failures {
            println!("    • {}", crate::render::color::yellow(f, config));
        }
    }

    // Record the final outcome on the session so the report reflects it even on
    // the interrupted / panic path.
    session.final_outcome = Some(outcome.clone());

    let report_path =
        super::report::save_session_report_with_recovery(&session, &outcome, &drain_failures);

    if is_json {
        print_json_outcome(&session, &outcome, report_path.as_deref(), &drain_failures);
    } else if let Some(path) = &report_path {
        // Re-print the path under the verdict so users see where to find it.
        println!(
            "  {} {}",
            crate::render::color::dim("Saved report:", config),
            crate::render::color::dim(&path.display().to_string(), config),
        );
    }

    let code = outcome.exit_code();

    // If the loop panicked, re-raise the failure as exit 101 AFTER cleanup so
    // the operator sees the standard panic exit code, having had the network
    // restored first.
    if panicked {
        std::process::exit(101);
    }

    code
}

/// Best-effort remaining-failure set for an interrupted run: the actionable
/// failures from the most recent diagnostics snapshot, or empty if none ran.
fn remaining_after_interrupt(session: &Session) -> Vec<DiagnosticKey> {
    session
        .snapshots
        .last()
        .map(|s| actionable_failures(&s.results).into_iter().collect())
        .unwrap_or_default()
}

fn print_json_outcome(
    session: &Session,
    outcome: &FinalOutcome,
    report_path: Option<&std::path::Path>,
    recovery_needed: &[String],
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
        FinalOutcome::Interrupted(_) => "interrupted",
    };

    let remaining: Vec<&str> = match outcome {
        FinalOutcome::Partial(rs)
        | FinalOutcome::Exhausted(rs)
        | FinalOutcome::Timeout(rs)
        | FinalOutcome::UserDeclined(rs)
        | FinalOutcome::Interrupted(rs) => rs.iter().map(|k| diagnostic_key_str(*k)).collect(),
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

    let mut intermittent: Vec<&str> = session
        .intermittent
        .iter()
        .map(|k| diagnostic_key_str(*k))
        .collect();
    intermittent.sort_unstable();

    let value = json!({
        "action": "fix",
        "outcome": outcome_label,
        "exit_code": outcome.exit_code(),
        "iterations": session.snapshots.len().saturating_sub(1),
        "remaining_failures": remaining,
        "intermittent_failures": intermittent,
        "applied_actions": actions_json,
        "elapsed_seconds": session.elapsed().as_secs(),
        "report_path": report_path.map(|p| p.display().to_string()),
        "interrupted": matches!(outcome, FinalOutcome::Interrupted(_)),
        "manual_recovery_needed": recovery_needed,
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

#[cfg(test)]
mod loop_tests {
    use super::*;
    use crate::diagnostics::{DiagnosticResult, DiagnosticResults};
    use std::collections::VecDeque;

    /// Scripted diagnostics: each probe pops the next pre-built result set.
    /// Panics if the loop consumes more probes than the test scripted — that
    /// panic IS an assertion on the loop's probe count.
    struct ScriptedProbe {
        script: VecDeque<DiagnosticResults>,
    }

    impl ScriptedProbe {
        fn new(script: Vec<DiagnosticResults>) -> Self {
            Self {
                script: script.into(),
            }
        }
    }

    impl DiagProbe for ScriptedProbe {
        async fn probe(&mut self) -> DiagnosticResults {
            self.script
                .pop_front()
                .expect("ScriptedProbe ran dry — the loop probed more often than the test scripted")
        }
    }

    fn all_ok() -> DiagnosticResults {
        DiagnosticResults {
            timestamp: "test".to_string(),
            adapters: DiagnosticResult::ok("Adapters", "1 active"),
            interfaces: DiagnosticResult::ok("Network", "1 up"),
            gateway: DiagnosticResult::ok("Gateway", "reachable"),
            dns: DiagnosticResult::ok("DNS", "resolving"),
            public_ip: DiagnosticResult::ok("Internet", "203.0.113.1"),
            latency: DiagnosticResult::ok("Latency", "low"),
            speed: DiagnosticResult::skip("Speed", "skipped"),
            ports: DiagnosticResult::ok("Ports", "open"),
            interface_details: None,
            adapter_details: None,
            gateway_details: None,
            dns_details: None,
            public_ip_details: None,
            latency_details: None,
            speed_details: None,
            port_details: None,
            technician: None,
            timed_out: false,
        }
    }

    fn dns_failing() -> DiagnosticResults {
        let mut r = all_ok();
        r.dns = DiagnosticResult::fail("DNS", "DNS resolution failed");
        r
    }

    /// Gateway fine, but public IP + ports dark — the ISP-outage shape that
    /// `hard_block_detected` recognizes.
    fn isp_outage() -> DiagnosticResults {
        let mut r = all_ok();
        r.public_ip = DiagnosticResult::fail("Internet", "Cannot determine public IP");
        r.ports = DiagnosticResult::fail("Ports", "All tested ports blocked");
        r
    }

    fn quiet_config() -> Config {
        // JSON format keeps the loop non-interactive regardless of the test
        // runner's TTY, so no prompts and no terminal output.
        Config::new().with_json()
    }

    /// The core evidence-quality acceptance test: a failure on the first pass
    /// that does not reproduce on the second is transient — no repair plan,
    /// outcome Fixed, exactly two probes consumed.
    #[tokio::test]
    async fn transient_blip_is_fixed_without_actions() {
        let mut probe = ScriptedProbe::new(vec![dns_failing(), all_ok()]);
        let config = quiet_config();
        let mut session = Session::new();
        let restore = RestoreRegistry::new();

        let outcome = run_with_probe(&mut probe, &config, &mut session, &restore).await;

        assert!(matches!(outcome, FinalOutcome::Fixed), "got {:?}", outcome);
        assert!(
            session.action_log.is_empty(),
            "no repair may run on unconfirmed evidence"
        );
        assert!(probe.script.is_empty(), "exactly two probes expected");
        assert!(session.baseline_confirmation.is_some());
        assert!(session.intermittent.contains(&DiagnosticKey::Dns));
    }

    /// A hard-block shape present on both passes short-circuits before any
    /// action.
    #[tokio::test]
    async fn confirmed_hard_block_short_circuits() {
        let mut probe = ScriptedProbe::new(vec![isp_outage(), isp_outage()]);
        let config = quiet_config();
        let mut session = Session::new();
        let restore = RestoreRegistry::new();

        let outcome = run_with_probe(&mut probe, &config, &mut session, &restore).await;

        assert!(
            matches!(outcome, FinalOutcome::HardBlock(HardBlock::IspOutage)),
            "got {:?}",
            outcome
        );
        assert!(session.action_log.is_empty());
    }

    /// A confirmed failure with every action's attempts pre-exhausted proves
    /// the loop plans against the CONFIRMED set (not the raw second-pass set)
    /// and reaches Exhausted without any apply IO.
    #[tokio::test]
    async fn confirmed_failure_with_no_actions_left_is_exhausted() {
        let mut probe = ScriptedProbe::new(vec![dns_failing(), dns_failing()]);
        let config = quiet_config();
        let mut session = Session::new();
        for action in action::all_actions() {
            session.attempts.insert(action.id, u8::MAX);
        }
        let restore = RestoreRegistry::new();

        let outcome = run_with_probe(&mut probe, &config, &mut session, &restore).await;

        match outcome {
            FinalOutcome::Exhausted(remaining) => {
                assert_eq!(remaining, vec![DiagnosticKey::Dns]);
            }
            other => panic!("expected Exhausted, got {:?}", other),
        }
        assert!(session.action_log.is_empty());
    }

    /// A healthy baseline ends the run after a single probe — no confirmation
    /// pass when there is nothing to confirm.
    #[tokio::test]
    async fn healthy_baseline_fixed_after_one_probe() {
        let mut probe = ScriptedProbe::new(vec![all_ok()]);
        let config = quiet_config();
        let mut session = Session::new();
        let restore = RestoreRegistry::new();

        let outcome = run_with_probe(&mut probe, &config, &mut session, &restore).await;

        assert!(matches!(outcome, FinalOutcome::Fixed));
        assert!(probe.script.is_empty(), "exactly one probe expected");
        assert!(session.baseline_confirmation.is_none());
        assert!(session.action_log.is_empty());
    }
}
