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
//!   composes the three above. A failing baseline is re-confirmed with a
//!   second diagnostic pass before the first repair plan.
//!
//! The platform-specific primitives (service restart, interface bounce, deep
//! stack reset) live in [`stages`] and are wrapped as [`Action`]s in
//! [`action::all_actions`]. The legacy fixed Stage 1/2/3 orchestrators were
//! removed in v3.4.0.

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

/// Step-success printer used by the platform primitives and the VPN
/// re-enable flow. The loop's plain-language output lives in
/// [`session::Reporter`].
pub fn print_step_ok(label: &str, config: &Config) {
    println!(
        "  {} {}",
        color::green(success_icon(config), config),
        color::green(label, config),
    );
}

/// Step-failure printer. See [`print_step_ok`].
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
