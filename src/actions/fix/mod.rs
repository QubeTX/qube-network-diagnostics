//! Diagnostic-driven fix loop entry point.
//!
//! `nd300 fix` (and the legacy `nd300 -f` flag form) lands here. The actual
//! work is split across:
//!
//! - [`action`] — the [`Action`] type system and registry of every fix
//!   primitive available on the current platform.
//! - [`triage`] — pure planning logic: pick which actions to apply for the
//!   current failure set, ordered by cost/risk.
//! - [`session`] — per-run state (attempts, effectiveness, snapshots) and the
//!   plain-language Reporter that drives terminal output.
//! - [`loop_runner`] — the bounded triage → apply → re-test loop that
//!   composes the three above.
//!
//! Stages 1 / 2 / 3 of the previous design have been replaced by the
//! diagnostic-driven loop. The platform-specific primitives the old stages
//! called still live in [`stages`] and are wrapped as [`Action`]s in
//! [`action::all_actions`].

pub mod action;
pub mod adapters;
pub mod arp;
pub mod cmd;
pub mod connectivity;
pub mod dhcp;
pub mod dns;
pub mod loop_runner;
pub mod report;
pub mod session;
pub mod stages;
pub mod triage;
pub mod vpn;
pub mod wifi;

use crate::config::Config;
use crate::render::color;

use super::{fail_icon, success_icon};

/// Legacy step-result type used by the platform primitives in [`stages`]
/// (interface bounce, Stage 3 stack reset). Retained during the v3 transition
/// so we don't have to rewrite every primitive at once. The triage loop
/// itself does not use this type directly.
pub struct StepResult {
    pub name: &'static str,
    pub success: bool,
    pub message: String,
}

/// Legacy step-success printer. Used by primitives in [`stages`]. The new
/// loop's plain-language output lives in [`session::Reporter`].
pub fn print_step_ok(label: &str, config: &Config) {
    println!(
        "  {} {}",
        color::green(success_icon(config), config),
        color::green(label, config),
    );
}

/// Legacy step-failure printer. See [`print_step_ok`].
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

/// Entry point for `nd300 fix` / `nd300 -f`. Dispatches to the diagnostic-
/// driven triage loop.
///
/// `_args` is currently a placeholder — the `--yes` flag lives at the
/// top-level of `Nd300Cli` (global). Future fix-only options will be threaded
/// through here.
pub async fn run(config: &Config, _args: crate::cli::FixArgs) -> i32 {
    loop_runner::run_and_finalize(config).await
}
