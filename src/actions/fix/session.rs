//! Session state + plain-language reporter for the fix loop.

use std::time::{Duration, Instant};

use crate::config::Config;
use crate::diagnostics::DiagnosticResults;
use crate::render::color;

use super::action::{Action, ActionOutcome, DiagnosticKey, Risk};
use super::triage::{Attempts, Effectiveness, HardBlock};

/// Hard wall-clock cap for a single `nd300 fix` run. Combined with the
/// per-iteration count and per-action attempt caps, this guarantees the loop
/// always exits in bounded time.
pub const WALL_CLOCK_CAP: Duration = Duration::from_secs(240);

/// Stabilization delay between iterations after non-disruptive actions. The
/// loop additionally honors per-action `Action::stabilization` for actions
/// that need longer (DHCP renew, interface bounce).
pub const DEFAULT_ITERATION_DELAY: Duration = Duration::from_secs(2);

/// Final outcome categories. Drives the verdict line and the "what to try
/// next" suggestions.
#[derive(Debug, Clone)]
pub enum FinalOutcome {
    /// Every actionable failure cleared.
    Fixed,
    /// Some failures cleared but not all.
    Partial(Vec<DiagnosticKey>),
    /// No more actions to try and failures remain.
    Exhausted(Vec<DiagnosticKey>),
    /// Loop exited cleanly because the failure cannot be auto-fixed.
    HardBlock(HardBlock),
    /// Iteration count or wall clock cap reached.
    Timeout(Vec<DiagnosticKey>),
    /// User declined a high-risk prompt; loop stopped with remaining failures.
    UserDeclined(Vec<DiagnosticKey>),
    /// Pre-flight check failed (e.g. not elevated). No diagnostics ran.
    PreflightFailed(String),
}

impl FinalOutcome {
    pub fn exit_code(&self) -> i32 {
        match self {
            FinalOutcome::Fixed => 0,
            FinalOutcome::Partial(_) => 1,
            FinalOutcome::Exhausted(_) => 2,
            FinalOutcome::HardBlock(_) => 1,
            FinalOutcome::Timeout(_) => 2,
            FinalOutcome::UserDeclined(_) => 1,
            FinalOutcome::PreflightFailed(_) => 2,
        }
    }
}

/// One row in the iteration timeline — what happened during one pass through
/// the loop.
#[derive(Debug, Clone)]
pub struct ActionRecord {
    pub action_id: super::action::ActionId,
    pub label: &'static str,
    pub outcome: ActionOutcome,
    pub duration: Duration,
    pub iteration: u8,
    /// Set when the user declined a high-risk prompt for this action.
    pub user_declined: bool,
    /// Set when the action was skipped because we couldn't render an
    /// interactive prompt (e.g. JSON mode + High-risk action).
    pub skipped_no_interaction: bool,
}

#[derive(Debug)]
pub struct IterationSnapshot {
    pub iteration: u8,
    pub results: DiagnosticResults,
}

#[derive(Debug)]
pub struct Session {
    pub started_at: Instant,
    pub baseline: Option<DiagnosticResults>,
    pub snapshots: Vec<IterationSnapshot>,
    pub action_log: Vec<ActionRecord>,
    pub attempts: Attempts,
    pub effectiveness: Effectiveness,
    pub vpn_names: Vec<String>,
    pub final_outcome: Option<FinalOutcome>,
}

impl Session {
    pub fn new() -> Self {
        Self {
            started_at: Instant::now(),
            baseline: None,
            snapshots: Vec::new(),
            action_log: Vec::new(),
            attempts: Attempts::new(),
            effectiveness: Effectiveness::new(),
            vpn_names: Vec::new(),
            final_outcome: None,
        }
    }

    pub fn elapsed(&self) -> Duration {
        self.started_at.elapsed()
    }

    pub fn wall_clock_exhausted(&self) -> bool {
        self.elapsed() >= WALL_CLOCK_CAP
    }

    pub fn record_baseline(&mut self, results: DiagnosticResults) {
        self.baseline = Some(results.clone());
        self.snapshots.push(IterationSnapshot {
            iteration: 0,
            results,
        });
    }

    pub fn record_iteration(&mut self, iteration: u8, results: DiagnosticResults) {
        self.snapshots.push(IterationSnapshot {
            iteration,
            results,
        });
    }

    pub fn record_action(
        &mut self,
        iteration: u8,
        action: &Action,
        outcome: ActionOutcome,
        duration: Duration,
        user_declined: bool,
        skipped_no_interaction: bool,
    ) {
        let entry = self.attempts.entry(action.id).or_insert(0);
        if !skipped_no_interaction && !user_declined {
            *entry = entry.saturating_add(1);
        }
        self.action_log.push(ActionRecord {
            action_id: action.id,
            label: action.label,
            outcome,
            duration,
            iteration,
            user_declined,
            skipped_no_interaction,
        });
    }

    /// After an iteration: compare prior failures to current ones; for each
    /// action applied this iteration, mark which targets newly cleared.
    pub fn update_effectiveness(
        &mut self,
        iteration: u8,
        prior_failures: &std::collections::HashSet<DiagnosticKey>,
        current_failures: &std::collections::HashSet<DiagnosticKey>,
    ) {
        let cleared: std::collections::HashSet<DiagnosticKey> = prior_failures
            .difference(current_failures)
            .copied()
            .collect();
        if cleared.is_empty() {
            return;
        }
        // Attribute clears to the last action in this iteration that targeted them.
        for record in self.action_log.iter().rev() {
            if record.iteration != iteration {
                break;
            }
            if !record.outcome.ok {
                continue;
            }
            // Find the registry action so we can read its targets.
            // (The label match is sufficient — ActionIds are unique.)
            for k in cleared.iter() {
                self.effectiveness
                    .insert((record.action_id, *k), true);
            }
        }
    }
}

// ── Reporter ────────────────────────────────────────────────────────────────

/// Plain-language reporter for terminal output. Uses the same Config as the
/// rest of nd300 so colors / ASCII / verbose are honored consistently.
pub struct Reporter<'a> {
    pub config: &'a Config,
}

impl<'a> Reporter<'a> {
    pub fn new(config: &'a Config) -> Self {
        Self { config }
    }

    pub fn header(&self) {
        println!();
        println!(
            "  {} {}",
            color::cyan("nd300 fix", self.config),
            color::dim("— diagnostic-driven recovery", self.config),
        );
    }

    pub fn iteration_header(&self, iteration: u8) {
        println!();
        println!(
            "  {} {}",
            color::cyan(&format!("Iteration {}", iteration), self.config),
            color::dim(
                "— running diagnostics, then applying targeted fixes",
                self.config,
            ),
        );
    }

    pub fn baseline_summary(&self, failure_count: usize) {
        if failure_count == 0 {
            println!(
                "  {} {}",
                color::green("✓", self.config),
                color::green("All diagnostics passing — nothing to fix.", self.config),
            );
        } else {
            println!(
                "  {} found {} failing area{}",
                color::yellow("→", self.config),
                failure_count,
                if failure_count == 1 { "" } else { "s" },
            );
        }
    }

    pub fn announce_action(&self, action: &Action) {
        println!();
        println!(
            "  {} {}",
            color::dim("•", self.config),
            color::dim(action.one_line_why, self.config),
        );
        print!(
            "  {} {} ",
            color::cyan("→", self.config),
            action.label,
        );
        use std::io::Write;
        let _ = std::io::stdout().flush();
    }

    pub fn finish_action(&self, outcome: &ActionOutcome, duration: Duration) {
        if outcome.ok {
            println!(
                "{} {}",
                color::green("✓", self.config),
                color::dim(
                    &format!("({:.1}s) {}", duration.as_secs_f64(), outcome.message),
                    self.config,
                ),
            );
        } else {
            println!(
                "{} {}",
                color::red("✗", self.config),
                color::red(&outcome.message, self.config),
            );
        }
    }

    pub fn finish_action_skipped(&self, reason: &str) {
        println!(
            "{} {}",
            color::yellow("·", self.config),
            color::yellow(reason, self.config),
        );
    }

    /// Render the structured high-risk action prompt and read Y/N from stdin.
    /// Returns `true` only on an explicit `y` / `Y` / `yes`. Anything else,
    /// including an empty press, is treated as N.
    pub fn high_risk_prompt(&self, action: &Action) -> bool {
        let exp = match &action.risk {
            Risk::High(e) => e,
            _ => return true, // non-High shouldn't reach here
        };

        let header = format!("Escalating: {}", exp.what);
        let bar = if self.config.use_unicode { '─' } else { '-' };
        let rule: String = std::iter::repeat(bar).take(76).collect();

        println!();
        println!(
            "  {}",
            color::yellow(&format!("┌─ {} ", header), self.config)
        );
        println!("  {}", color::dim(&rule, self.config));
        println!(
            "  {}",
            color::bold("Why I want to do this:", self.config)
        );
        for line in wrap_text(exp.why, 70) {
            println!("    {}", line);
        }
        println!();
        println!("  {}", color::bold("What will happen:", self.config));
        for bullet in exp.side_effects {
            println!("    • {}", bullet);
        }
        println!();
        println!(
            "  {} {}",
            color::bold("Reversible:", self.config),
            exp.reversible.label(),
        );
        println!(
            "  {} {}",
            color::bold("Typical duration:", self.config),
            exp.typical_duration,
        );
        println!("  {}", color::dim(&rule, self.config));

        // Strict Y/N — empty input or any non-y answer is treated as No.
        use std::io::Write;
        print!(
            "  {} ",
            color::yellow("Continue? Type 'y' to proceed, anything else to skip:", self.config),
        );
        let _ = std::io::stdout().flush();

        let mut input = String::new();
        if std::io::stdin().read_line(&mut input).is_err() {
            return false;
        }
        matches!(input.trim(), "y" | "Y" | "yes" | "Yes" | "YES")
    }

    pub fn high_risk_skipped_no_tty(&self, action: &Action) {
        println!(
            "  {} {}",
            color::yellow("·", self.config),
            color::yellow(
                &format!(
                    "Skipped {}: this action requires interactive confirmation. Re-run `nd300 fix` in a terminal to attempt it.",
                    action.label,
                ),
                self.config,
            ),
        );
    }

    pub fn high_risk_declined(&self, action: &Action) {
        println!(
            "  {} {}",
            color::yellow("·", self.config),
            color::dim(
                &format!("Skipped {} (you declined the prompt).", action.label),
                self.config,
            ),
        );
    }

    pub fn final_verdict(&self, outcome: &FinalOutcome, report_path: Option<&std::path::Path>) {
        let bar = if self.config.use_unicode { '─' } else { '-' };
        let rule: String = std::iter::repeat(bar).take(50).collect();

        println!();
        println!(
            "  {} {}",
            color::bold("Result", self.config),
            color::dim(&rule, self.config),
        );

        match outcome {
            FinalOutcome::Fixed => {
                println!(
                    "  {} {}",
                    color::green("✓ Fixed", self.config),
                    color::green(
                        "Connectivity is healthy now. The actions above resolved the failures.",
                        self.config,
                    ),
                );
            }
            FinalOutcome::Partial(remaining) => {
                println!(
                    "  {} {}",
                    color::yellow("⚠ Partially fixed", self.config),
                    color::yellow(
                        &format!("{} remain{}", describe_keys(remaining), if remaining.len() == 1 { "s" } else { "" }),
                        self.config,
                    ),
                );
                self.suggestions_for(remaining);
            }
            FinalOutcome::Exhausted(remaining) => {
                println!(
                    "  {} {}",
                    color::red("✗ Couldn't fix", self.config),
                    color::red(
                        &format!("Tried every applicable action; {} still failing", describe_keys(remaining)),
                        self.config,
                    ),
                );
                self.suggestions_for(remaining);
            }
            FinalOutcome::HardBlock(reason) => {
                println!(
                    "  {} {}",
                    color::yellow("⚠ Cannot fix from here", self.config),
                    color::yellow(&reason.user_message(), self.config),
                );
            }
            FinalOutcome::Timeout(remaining) => {
                println!(
                    "  {} {}",
                    color::red("✗ Timed out", self.config),
                    color::red(
                        &format!("Loop hit its safety cap; {} still failing", describe_keys(remaining)),
                        self.config,
                    ),
                );
                self.suggestions_for(remaining);
            }
            FinalOutcome::UserDeclined(remaining) => {
                println!(
                    "  {} {}",
                    color::yellow("⚠ Stopped at your request", self.config),
                    color::yellow(
                        &format!("You declined a high-risk action; {} still failing", describe_keys(remaining)),
                        self.config,
                    ),
                );
                self.suggestions_for(remaining);
            }
            FinalOutcome::PreflightFailed(reason) => {
                println!(
                    "  {} {}",
                    color::red("✗ Could not start", self.config),
                    color::red(reason, self.config),
                );
            }
        }

        if let Some(path) = report_path {
            println!(
                "  {} {}",
                color::dim("Full report:", self.config),
                color::dim(&path.display().to_string(), self.config),
            );
        }
    }

    fn suggestions_for(&self, remaining: &[DiagnosticKey]) {
        if remaining.is_empty() {
            return;
        }
        println!();
        println!("  {}", color::bold("What to try next:", self.config));
        for s in suggestions_for_keys(remaining) {
            println!("    • {}", s);
        }
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

/// Map remaining failure keys to concrete plain-language suggestions.
pub fn suggestions_for_keys(remaining: &[DiagnosticKey]) -> Vec<String> {
    use DiagnosticKey::*;
    let mut out: Vec<String> = Vec::new();

    let has = |k: DiagnosticKey| remaining.contains(&k);

    if has(Adapters) || has(Interfaces) {
        out.push(
            "Reboot your computer — a deeper hardware/driver state may need a clean start."
                .to_string(),
        );
        out.push(
            "Check for network driver updates from your hardware vendor (Intel, Realtek, etc.)."
                .to_string(),
        );
    }
    if has(Gateway) {
        out.push("Power-cycle your router / modem (unplug for 30 seconds, plug back in).".to_string());
        out.push("Try a different cable or Wi-Fi network if available.".to_string());
    }
    if has(Dns) {
        out.push("Try `nd300 fix` again, then choose `Switch DNS to Cloudflare` if it offers it.".to_string());
        out.push("Check your router admin page for a custom DNS setting and remove it.".to_string());
    }
    if has(PublicIp) {
        out.push("Check your ISP's status page — there may be an outage in your area.".to_string());
        out.push("Disconnect any VPN you have running (including work VPNs) and re-test.".to_string());
    }
    if has(Latency) {
        out.push("If on Wi-Fi: move closer to your router or try a 5 GHz network.".to_string());
        out.push("Run a speed test from another device on the same network to compare.".to_string());
    }
    if has(Ports) {
        out.push(
            "Outbound ports may be blocked by a firewall (work network or AV software). Contact IT or check your firewall rules.".to_string(),
        );
    }

    if out.is_empty() {
        out.push("Reboot the machine and try again.".to_string());
        out.push("Run `nd300 -t` for the full technician report and share it with support.".to_string());
    }
    out
}

/// Wrap a text block to the given column width without splitting words.
fn wrap_text(text: &str, width: usize) -> Vec<String> {
    let mut lines = Vec::new();
    let mut current = String::new();
    for word in text.split_whitespace() {
        if current.len() + word.len() + 1 > width && !current.is_empty() {
            lines.push(std::mem::take(&mut current));
        }
        if !current.is_empty() {
            current.push(' ');
        }
        current.push_str(word);
    }
    if !current.is_empty() {
        lines.push(current);
    }
    lines
}
