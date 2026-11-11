//! Session state + plain-language reporter for the fix loop.

use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::sync::Mutex;

use crate::config::Config;
use crate::diagnostics::DiagnosticResults;
use crate::render::color;

use super::action::{Action, ActionOutcome, DiagnosticKey, Risk};
use super::triage::{Attempts, Effectiveness, HardBlock};
use super::vpn::DisabledVpn;

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
    /// User declined a confirmation prompt; loop stopped with remaining failures.
    UserDeclined(Vec<DiagnosticKey>),
    /// Pre-flight check failed (e.g. not elevated). No diagnostics ran.
    PreflightFailed(String),
    /// The run was interrupted before it could finish — Ctrl-C, SIGTERM,
    /// SIGHUP, a caught panic, or the outer drain timeout. The restore registry is drained on
    /// this path so any half-applied network state is rolled back; the carried
    /// keys are whatever failures were still outstanding when the interrupt
    /// landed (best-effort, may be empty if the interrupt arrived early).
    Interrupted(Vec<DiagnosticKey>),
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
            // 130 = process terminated by SIGINT, the conventional Ctrl-C code.
            FinalOutcome::Interrupted(_) => 130,
        }
    }
}

// ── Restore registry ──────────────────────────────────────────────────────────

/// Per-op timeout for a single restore operation during the drain. Restores
/// run best-effort on a terminal path (normal end, Ctrl-C, panic, or the outer
/// drain cap), so each one is individually bounded to keep the drain prompt.
const RESTORE_OP_TIMEOUT: Duration = Duration::from_secs(30);

/// Scheduling/reporting headroom added after reserving one complete per-op
/// timeout for every pending restore. This replaces the old fixed 90-second
/// aggregate cap, which could prevent the fourth and later LIFO entries from
/// receiving any attempt at all.
const RESTORE_DRAIN_MARGIN: Duration = Duration::from_secs(5);

/// An inverse operation that undoes a destructive change applied during the fix.
///
/// Destructive actions register the matching `RestoreOp` *before* mutating
/// state and `mark_resolved` it once they have restored the state themselves
/// on a normal path. Anything still unresolved when the run ends — whether it
/// ended normally, via a supported signal, via a caught panic, or via the wall-clock cap —
/// is replayed by [`RestoreRegistry::drain`].
#[derive(Debug, Clone)]
pub enum RestoreOp {
    /// Bring a network interface back up (idempotent — safe if already up).
    ReEnableInterface { iface: String },
    /// Re-connect a consumer VPN that the fix disabled.
    ReEnableVpn(Arc<DisabledVpn>),
    /// Restore exact resolver state if a macOS DNS command is interrupted.
    #[cfg(target_os = "macos")]
    RestoreMacosDns(Arc<super::dns::MacosDnsSnapshot>),
    /// Re-enable a macOS network service and restore exact DNS/search-domain
    /// state. The hardened deep-reset path never deletes/recreates services.
    #[cfg(target_os = "macos")]
    ReEnableMacosService {
        iface: String,
        service: String,
        dns_snapshot: Arc<super::dns::MacosDnsSnapshot>,
    },
}

impl RestoreOp {
    /// Short human-readable label for reports / manual-recovery guidance.
    fn label(&self) -> String {
        match self {
            RestoreOp::ReEnableInterface { iface } => {
                format!("re-enable network adapter {}", iface)
            }
            RestoreOp::ReEnableVpn(vpn) => format!("re-enable VPN {}", vpn.name),
            #[cfg(target_os = "macos")]
            RestoreOp::RestoreMacosDns(snapshot) => {
                format!("restore DNS/search domains on {}", snapshot.service())
            }
            #[cfg(target_os = "macos")]
            RestoreOp::ReEnableMacosService { service, .. } => {
                format!("re-enable macOS network service {}", service)
            }
        }
    }
}

/// Run a single restore operation. Returns `Ok(())` on success, `Err(reason)`
/// on failure — the caller turns failures into manual-recovery guidance.
async fn restore_op(op: &RestoreOp) -> Result<(), String> {
    match op {
        RestoreOp::ReEnableInterface { iface } => super::stages::restore_interface(iface).await,
        RestoreOp::ReEnableVpn(vpn) => {
            if super::vpn::reenable_vpn(vpn).await {
                Ok(())
            } else {
                Err(format!(
                    "could not positively verify VPN {} reconnected",
                    vpn.name
                ))
            }
        }
        #[cfg(target_os = "macos")]
        RestoreOp::RestoreMacosDns(snapshot) => {
            super::dns::restore_macos_dns_snapshot(snapshot).await
        }
        #[cfg(target_os = "macos")]
        RestoreOp::ReEnableMacosService {
            iface,
            service,
            dns_snapshot,
        } => super::stages::restore_macos_service_state(iface, service, dns_snapshot).await,
    }
}

/// One entry in the restore registry: an inverse op plus whether the owning
/// action already resolved it on a normal path.
#[derive(Debug, Clone)]
struct RegisteredOp {
    op: RestoreOp,
    resolved: bool,
}

/// Token identifying a registered restore op so the owning action can mark it
/// resolved once it has restored the state itself.
pub type RestoreToken = usize;

/// Tracks the inverse of every destructive change the fix has applied, so any
/// terminal path can roll back half-applied state.
///
/// Cheaply cloneable (it is an `Arc` around a single mutex), so the same
/// registry can be shared into the loop future and the Ctrl-C / drain arm of
/// the outer `select!`. The mutex is `tokio::sync::Mutex`, which is
/// **non-poisoning** — a panic inside the loop (caught by `catch_unwind`) does
/// not wedge the registry, so the post-panic drain still works.
#[derive(Clone, Default)]
pub struct RestoreRegistry {
    inner: Arc<Mutex<Vec<RegisteredOp>>>,
}

impl RestoreRegistry {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(Mutex::new(Vec::new())),
        }
    }

    /// Register an inverse op and return a token to resolve it later. Call this
    /// *before* applying the matching destructive change.
    pub async fn register(&self, op: RestoreOp) -> RestoreToken {
        let mut guard = self.inner.lock().await;
        guard.push(RegisteredOp {
            op,
            resolved: false,
        });
        guard.len() - 1
    }

    /// Mark a previously-registered op resolved — the owning action restored the
    /// state itself, so the drain should skip it.
    pub async fn mark_resolved(&self, token: RestoreToken) {
        let mut guard = self.inner.lock().await;
        if let Some(entry) = guard.get_mut(token) {
            entry.resolved = true;
        }
    }

    /// Aggregate timeout for a drain snapshot. Reserve a full bounded attempt
    /// for every currently-pending inverse operation, plus small bookkeeping
    /// headroom. The fix loop has stopped mutating state before it asks for
    /// this value, so the following drain sees the same or fewer entries.
    pub(crate) async fn required_drain_timeout(&self) -> Duration {
        let pending = self
            .inner
            .lock()
            .await
            .iter()
            .filter(|entry| !entry.resolved)
            .count();
        let pending = u64::try_from(pending).unwrap_or(u64::MAX);
        let operation_budget = RESTORE_OP_TIMEOUT.as_secs().saturating_mul(pending);
        Duration::from_secs(operation_budget.saturating_add(RESTORE_DRAIN_MARGIN.as_secs()))
    }

    /// Run every still-unresolved restore op, best-effort. Snapshots the
    /// pending ops under the lock, releases the lock, then runs each with its
    /// own timeout so a wedged restore can't stall the others. Returns a
    /// human-readable failure string per op that could not be restored
    /// (empty `Vec` = everything restored cleanly / nothing to do).
    ///
    /// Resolved ops are cleared from the registry as they succeed, so a second
    /// drain (should one ever happen) is a no-op for already-restored state.
    pub async fn drain(&self) -> Vec<String> {
        // Snapshot pending ops + their indices under the lock, then release it
        // so the restore subprocess calls don't hold the mutex.
        let pending: Vec<(usize, RestoreOp)> = {
            let guard = self.inner.lock().await;
            guard
                .iter()
                .enumerate()
                .rev()
                .filter(|(_, e)| !e.resolved)
                .map(|(i, e)| (i, e.op.clone()))
                .collect()
        };

        let mut failures = Vec::new();
        for (idx, op) in pending {
            let result = match tokio::time::timeout(RESTORE_OP_TIMEOUT, restore_op(&op)).await {
                Ok(Ok(())) => Ok(()),
                Ok(Err(e)) => Err(e),
                Err(_) => Err(format!("timed out after {}s", RESTORE_OP_TIMEOUT.as_secs())),
            };
            match result {
                Ok(()) => {
                    // Mark resolved so a re-drain won't repeat it.
                    let mut guard = self.inner.lock().await;
                    if let Some(entry) = guard.get_mut(idx) {
                        entry.resolved = true;
                    }
                }
                Err(reason) => {
                    failures.push(format!("Could not {}: {}", op.label(), reason));
                }
            }
        }
        failures
    }

    /// Number of still-unresolved restore ops. Test-only inspection helper.
    #[cfg(test)]
    pub(crate) async fn pending_count(&self) -> usize {
        self.inner
            .lock()
            .await
            .iter()
            .filter(|e| !e.resolved)
            .count()
    }
}

/// One row in the iteration timeline — what happened during one pass through
/// the loop.
#[derive(Debug, Clone)]
pub struct ActionRecord {
    pub action_id: super::action::ActionId,
    pub label: &'static str,
    /// The failure categories this action declares it can address — used by
    /// effectiveness attribution so an action is only credited for clearing
    /// failures it actually targets.
    pub targets: &'static [DiagnosticKey],
    pub outcome: ActionOutcome,
    pub duration: Duration,
    pub iteration: u8,
    /// Set when the user declined a confirmation prompt for this action.
    pub user_declined: bool,
    /// Set when the action was skipped because we couldn't render an
    /// interactive prompt (e.g. JSON mode + confirmation-gated action).
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
    /// The second baseline pass used to confirm the failure set before the
    /// first repair plan. Kept separate from `snapshots` so the report's
    /// "final snapshot" and the JSON `iterations` count keep their meaning.
    pub baseline_confirmation: Option<DiagnosticResults>,
    /// Failures seen in exactly one of the two baseline passes — transient
    /// flickers. Their later "recoveries" earn no effectiveness credit.
    pub intermittent: std::collections::HashSet<DiagnosticKey>,
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
            baseline_confirmation: None,
            intermittent: std::collections::HashSet::new(),
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
        self.snapshots
            .push(IterationSnapshot { iteration, results });
    }

    /// Record the second baseline pass and the failures that flickered
    /// between the two passes.
    pub fn record_confirmation(
        &mut self,
        results: DiagnosticResults,
        intermittent: std::collections::HashSet<DiagnosticKey>,
    ) {
        self.baseline_confirmation = Some(results);
        self.intermittent = intermittent;
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
            targets: action.targets,
            outcome,
            duration,
            iteration,
            user_declined,
            skipped_no_interaction,
        });
    }

    /// After an iteration: compare prior failures to current ones; credit each
    /// newly-cleared failure to the **most recent successful action this
    /// iteration whose declared targets contain it**. Failures flagged
    /// intermittent at baseline earn no credit — natural recoveries must not
    /// bias future planning toward unrelated actions.
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
        for key in cleared {
            if self.intermittent.contains(&key) {
                continue;
            }
            if let Some(record) = self
                .action_log
                .iter()
                .rev()
                .take_while(|r| r.iteration == iteration)
                .find(|r| r.outcome.ok && r.targets.contains(&key))
            {
                self.effectiveness.insert((record.action_id, key), true);
            }
        }
    }
}

impl Default for Session {
    fn default() -> Self {
        Self::new()
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

    /// Announce the second baseline pass that double-checks the failure set.
    pub fn confirmation_pass(&self) {
        println!(
            "  {} {}",
            color::dim("→", self.config),
            color::dim(
                "Double-checking the failures with a second diagnostic pass...",
                self.config,
            ),
        );
    }

    /// Report how the confirmation pass shook out.
    pub fn confirmation_result(&self, confirmed: usize, transient: usize) {
        if confirmed == 0 && transient > 0 {
            println!(
                "  {} {}",
                color::green("✓", self.config),
                color::green(
                    "The failures did not reproduce on the second pass — they were transient.",
                    self.config,
                ),
            );
        } else if transient > 0 {
            println!(
                "  {} {} confirmed, {} transient (will not be acted on)",
                color::yellow("→", self.config),
                confirmed,
                transient,
            );
        } else {
            println!(
                "  {} {} failure{} confirmed on both passes",
                color::yellow("→", self.config),
                confirmed,
                if confirmed == 1 { "" } else { "s" },
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
        print!("  {} {} ", color::cyan("→", self.config), action.label,);
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

    /// Render a compact confirmation gate for mutating, non-high-risk actions.
    /// Returns `true` only on explicit y/yes.
    pub fn confirmation_prompt(&self, action: &Action) -> bool {
        println!();
        println!(
            "  {} {}",
            color::yellow("Confirm:", self.config),
            color::yellow(action.label, self.config),
        );
        println!("    {}", color::dim(action.one_line_why, self.config));
        println!(
            "    {}",
            color::dim("This changes live network settings. Use --yes to auto-confirm this class of action.", self.config),
        );

        use std::io::Write;
        print!(
            "  {} ",
            color::yellow(
                "Continue? Type 'y' to proceed, anything else to skip:",
                self.config
            ),
        );
        let _ = std::io::stdout().flush();

        let mut input = String::new();
        if std::io::stdin().read_line(&mut input).is_err() {
            return false;
        }
        matches!(input.trim(), "y" | "Y" | "yes" | "Yes" | "YES")
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
        let rule: String = std::iter::repeat_n(bar, 76).collect();

        println!();
        println!(
            "  {}",
            color::yellow(&format!("┌─ {} ", header), self.config)
        );
        println!("  {}", color::dim(&rule, self.config));
        println!("  {}", color::bold("Why I want to do this:", self.config));
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
            color::yellow(
                "Continue? Type 'y' to proceed, anything else to skip:",
                self.config
            ),
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
        self.confirmation_declined(action);
    }

    pub fn confirmation_declined(&self, action: &Action) {
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
        let rule: String = std::iter::repeat_n(bar, 50).collect();

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
                        &format!(
                            "{} remain{}",
                            describe_keys(remaining),
                            if remaining.len() == 1 { "s" } else { "" }
                        ),
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
                        &format!(
                            "Tried every applicable action; {} still failing",
                            describe_keys(remaining)
                        ),
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
                        &format!(
                            "Loop hit its safety cap; {} still failing",
                            describe_keys(remaining)
                        ),
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
                        &format!(
                            "You declined a confirmation prompt; {} still failing",
                            describe_keys(remaining)
                        ),
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
            FinalOutcome::Interrupted(_) => {
                println!(
                    "  {} {}",
                    color::yellow("⚠ Interrupted", self.config),
                    color::yellow(
                        "The fix was stopped before it finished. nd300 attempted to restore any network state it had changed — see below for anything that needs manual recovery.",
                        self.config,
                    ),
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
        out.push(
            "Power-cycle your router / modem (unplug for 30 seconds, plug back in).".to_string(),
        );
        out.push("Try a different cable or Wi-Fi network if available.".to_string());
    }
    if has(Dns) {
        out.push(
            "Try `nd300 fix` again, then choose `Switch DNS to Cloudflare` if it offers it."
                .to_string(),
        );
        out.push(
            "Check your router admin page for a custom DNS setting and remove it.".to_string(),
        );
    }
    if has(PublicIp) {
        out.push("Check your ISP's status page — there may be an outage in your area.".to_string());
        out.push(
            "Disconnect any VPN you have running (including work VPNs) and re-test.".to_string(),
        );
    }
    if has(Latency) {
        out.push("If on Wi-Fi: move closer to your router or try a 5 GHz network.".to_string());
        out.push(
            "Run a speed test from another device on the same network to compare.".to_string(),
        );
    }
    if has(Ports) {
        out.push(
            "Outbound ports may be blocked by a firewall (work network or AV software). Contact IT or check your firewall rules.".to_string(),
        );
    }

    if out.is_empty() {
        out.push("Reboot the machine and try again.".to_string());
        out.push(
            "Run `nd300 -t` for the full technician report and share it with support.".to_string(),
        );
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

#[cfg(test)]
mod restore_registry_tests {
    use super::*;
    use crate::actions::fix::vpn::{DisableMethod, DisabledVpn};

    /// A restore op that fails fast and deterministically: a vendor CLI whose
    /// binary does not exist, so `restore_op` returns a spawn error without
    /// touching any real network state.
    fn failing_vpn_op(name: &str) -> RestoreOp {
        RestoreOp::ReEnableVpn(Arc::new(DisabledVpn {
            name: name.to_string(),
            method: DisableMethod::VendorCli(
                "nd300-nonexistent-vpn-binary".to_string(),
                vec!["connect".to_string()],
            ),
        }))
    }

    #[tokio::test]
    async fn empty_registry_drains_to_no_failures() {
        let reg = RestoreRegistry::new();
        assert_eq!(reg.pending_count().await, 0);
        assert!(reg.drain().await.is_empty());
    }

    #[tokio::test]
    async fn register_then_resolve_clears_pending() {
        let reg = RestoreRegistry::new();
        let t1 = reg.register(failing_vpn_op("VpnA")).await;
        let _t2 = reg.register(failing_vpn_op("VpnB")).await;
        assert_eq!(reg.pending_count().await, 2);

        reg.mark_resolved(t1).await;
        assert_eq!(reg.pending_count().await, 1);
    }

    #[tokio::test]
    async fn drain_skips_resolved_and_reports_unresolved_failures() {
        let reg = RestoreRegistry::new();
        let resolved = reg.register(failing_vpn_op("ResolvedVpn")).await;
        let _pending = reg.register(failing_vpn_op("PendingVpn")).await;

        // The first op is restored by its owning action — mark it resolved so
        // the drain must skip it entirely.
        reg.mark_resolved(resolved).await;

        let failures = reg.drain().await;

        // Exactly one failure, for the still-unresolved op, and it must name
        // that VPN (proving the resolved one was skipped, not attempted).
        assert_eq!(failures.len(), 1, "failures: {:?}", failures);
        assert!(
            failures[0].contains("PendingVpn"),
            "unexpected failure text: {}",
            failures[0]
        );
        assert!(
            !failures[0].contains("ResolvedVpn"),
            "resolved op should not have been drained: {}",
            failures[0]
        );

        // After a drain, even failed ops are not retried as "pending success",
        // but the unresolved op stays unresolved (drain only marks resolved on
        // success), so a second drain re-attempts it and fails the same way.
        let second = reg.drain().await;
        assert_eq!(second.len(), 1);
    }

    #[tokio::test]
    async fn drain_restores_in_lifo_order() {
        let reg = RestoreRegistry::new();
        reg.register(failing_vpn_op("FirstRegistered")).await;
        reg.register(failing_vpn_op("LastRegistered")).await;

        let failures = reg.drain().await;

        assert_eq!(failures.len(), 2);
        assert!(failures[0].contains("LastRegistered"), "{failures:?}");
        assert!(failures[1].contains("FirstRegistered"), "{failures:?}");
    }

    #[tokio::test]
    async fn aggregate_timeout_reserves_an_attempt_for_every_pending_restore() {
        let reg = RestoreRegistry::new();
        for index in 0..4 {
            reg.register(failing_vpn_op(&format!("Vpn{index}"))).await;
        }

        assert_eq!(reg.required_drain_timeout().await, Duration::from_secs(125));
    }

    #[tokio::test]
    async fn drain_attempts_more_than_three_pending_ops_in_lifo_order() {
        let reg = RestoreRegistry::new();
        for index in 0..5 {
            reg.register(failing_vpn_op(&format!("Vpn{index}"))).await;
        }

        let failures = reg.drain().await;

        assert_eq!(failures.len(), 5, "failures: {failures:?}");
        for (position, expected_index) in (0..5).rev().enumerate() {
            assert!(
                failures[position].contains(&format!("Vpn{expected_index}")),
                "unexpected LIFO result: {failures:?}"
            );
        }
    }

    #[test]
    fn interrupted_exit_code_is_130() {
        assert_eq!(FinalOutcome::Interrupted(Vec::new()).exit_code(), 130);
    }
}

#[cfg(test)]
mod effectiveness_tests {
    use super::*;
    use crate::actions::fix::action::ActionId;
    use std::collections::HashSet;

    fn record(
        action_id: ActionId,
        targets: &'static [DiagnosticKey],
        ok: bool,
        iteration: u8,
    ) -> ActionRecord {
        ActionRecord {
            action_id,
            label: "test action",
            targets,
            outcome: if ok {
                ActionOutcome::ok("ok")
            } else {
                ActionOutcome::fail("fail")
            },
            duration: Duration::from_millis(1),
            iteration,
            user_declined: false,
            skipped_no_interaction: false,
        }
    }

    fn keys(ks: &[DiagnosticKey]) -> HashSet<DiagnosticKey> {
        ks.iter().copied().collect()
    }

    /// Pins the fix for the original bug: a successful action that does NOT
    /// target the cleared key must earn no credit for it.
    #[test]
    fn credit_requires_matching_targets() {
        let mut s = Session::new();
        s.action_log.push(record(
            ActionId::FlushArp,
            &[DiagnosticKey::Gateway],
            true,
            1,
        ));
        s.action_log
            .push(record(ActionId::FlushDns, &[DiagnosticKey::Dns], true, 1));

        s.update_effectiveness(1, &keys(&[DiagnosticKey::Dns]), &keys(&[]));

        assert_eq!(
            s.effectiveness
                .get(&(ActionId::FlushDns, DiagnosticKey::Dns)),
            Some(&true)
        );
        assert!(
            !s.effectiveness
                .contains_key(&(ActionId::FlushArp, DiagnosticKey::Dns)),
            "non-targeting action must not be credited"
        );
    }

    #[test]
    fn most_recent_targeting_action_wins() {
        let mut s = Session::new();
        s.action_log
            .push(record(ActionId::FlushDns, &[DiagnosticKey::Dns], true, 1));
        s.action_log.push(record(
            ActionId::SetDnsAutomatic,
            &[DiagnosticKey::Dns],
            true,
            1,
        ));

        s.update_effectiveness(1, &keys(&[DiagnosticKey::Dns]), &keys(&[]));

        assert!(s
            .effectiveness
            .contains_key(&(ActionId::SetDnsAutomatic, DiagnosticKey::Dns)));
        assert!(
            !s.effectiveness
                .contains_key(&(ActionId::FlushDns, DiagnosticKey::Dns)),
            "only the most recent targeting action is credited"
        );
    }

    #[test]
    fn failed_action_earns_no_credit() {
        let mut s = Session::new();
        s.action_log
            .push(record(ActionId::FlushDns, &[DiagnosticKey::Dns], false, 1));

        s.update_effectiveness(1, &keys(&[DiagnosticKey::Dns]), &keys(&[]));

        assert!(s.effectiveness.is_empty());
    }

    #[test]
    fn prior_iteration_records_never_credited() {
        let mut s = Session::new();
        s.action_log
            .push(record(ActionId::FlushDns, &[DiagnosticKey::Dns], true, 1));

        s.update_effectiveness(2, &keys(&[DiagnosticKey::Dns]), &keys(&[]));

        assert!(s.effectiveness.is_empty());
    }

    #[test]
    fn nothing_cleared_changes_nothing() {
        let mut s = Session::new();
        s.action_log
            .push(record(ActionId::FlushDns, &[DiagnosticKey::Dns], true, 1));

        s.update_effectiveness(
            1,
            &keys(&[DiagnosticKey::Dns]),
            &keys(&[DiagnosticKey::Dns]),
        );

        assert!(s.effectiveness.is_empty());
    }

    /// A failure flagged intermittent at baseline must not generate credit
    /// when it "clears" — that's the natural recovery the confirmation pass
    /// observed, not the action working.
    #[test]
    fn intermittent_key_earns_no_credit() {
        let mut s = Session::new();
        s.intermittent.insert(DiagnosticKey::Dns);
        s.action_log
            .push(record(ActionId::FlushDns, &[DiagnosticKey::Dns], true, 1));

        s.update_effectiveness(1, &keys(&[DiagnosticKey::Dns]), &keys(&[]));

        assert!(s.effectiveness.is_empty());
    }
}
