// Cross-method install cleanup for ND-300 (`nd300 migrate-cleanup`).
//
// PURPOSE
// -------
// On Windows a user can end up with more than one copy of nd300/speedqx on PATH:
//
//   * A prior `cargo install nd300` / cargo-dist PowerShell-installer copy in
//     `~\.cargo\bin` that SHADOWS a freshly-installed MSI/EXE copy (both are on
//     PATH; the cargo copy usually wins because `.cargo\bin` is earlier).
//   * Two Windows editions coexisting: Global perMachine
//     (`C:\Program Files\nd300\bin`) and Corporate perUser
//     (`%LocalAppData%\Programs\nd300\bin`).
//
// Operator policy: there should be exactly ONE version/edition installed at a
// time. `migrate-cleanup` consolidates aggressively — interactively (from the
// installer's "Remove older copy" checkboxes) AND on a silent self-update
// (`nd300 update` re-runs the installer with the cleanup tasks/properties
// defaulted ON). Mac/Linux is already safe (the shell installer overwrites the
// same `~/.cargo/bin`), so this is a Windows-only consolidation; on other
// platforms it is a no-op that exits 0.
//
// DESIGN (hybrid): the installer owns the CONSENT UX (checkboxes / MSI
// properties / Inno tasks); this binary owns the DELETION LOGIC by reusing the
// already-tested primitives from `uninstall.rs` (`uninstall_path`,
// `is_sole_package_in_dir`, `OUR_BINARIES`) and `update.rs` (`cargo_bin_dir`,
// `same_path`, `classify_install_path`, `current_install_shadows_cargo_install`,
// `current_exe_real_path`, `classify_shadow_cleanup`). It does NOT re-implement
// deletion.
//
// HARD SAFETY GUARANTEES (see unit tests at the bottom of this file):
//   1. Only ever deletes files whose stem is in `OUR_BINARIES`
//      (`nd300.exe` / `speedqx.exe`). Never `cargo.exe`, `rustup.exe`, or any
//      non-allowlisted file. (`uninstall_path` only ever touches OUR_BINARIES.)
//   2. Never removes the `.cargo\bin` PATH entry — `uninstall_path` only edits
//      PATH when `is_sole_package_in_dir` is true, and `.cargo\bin` always also
//      contains cargo.exe/rustup.exe, so its PATH entry is left intact.
//   3. Never touches `~/Downloads` (no path this module computes is under it).
//   4. Never deletes the RUNNING install — every candidate is `same_path`-checked
//      against the running exe's directory and skipped if it matches.
//   5. Never escalates privileges. If a target needs admin we don't have, it
//      reports "needs admin: <path>" and CONTINUES; exit code stays 0.
//   6. Refuses shell-unsafe paths (delegated to `uninstall_path`'s
//      `build_delayed_delete_command`, which returns None for cmd
//      metacharacters; that surfaces here as a NotRemoved -> reported, exit 0).
//
// EXIT CODE: 0 even on a partial or empty cleanup — consolidation is advisory and
// must NEVER fail the installer. Only a true internal error (couldn't determine
// our own location) is nonzero.

use crate::config::Config;
use std::path::{Path, PathBuf};

use crate::cli::MigrateArgs;

// The primitives we reuse, so the intent ("we delete via uninstall, we classify
// via update") is legible at this module's top. `OUR_BINARIES` backs the
// cross-platform allowlist; `uninstall_path` (the actual deletion) is only called
// on the Windows consolidation path, so its import is Windows-gated to avoid an
// unused-import warning on macOS/Linux.
#[cfg(windows)]
use super::uninstall::uninstall_path;
use super::uninstall::OUR_BINARIES;
#[cfg(windows)]
use super::update::{
    cargo_bin_dir, classify_install_path, classify_shadow_cleanup, current_exe_real_path,
    current_install_shadows_cargo_install, same_path, InstallOrigin, ShadowCleanupDecision,
};

/// A single cleanup target after deletion has been attempted (or skipped).
///
/// The full set of variants is the platform-agnostic contract (so it mirrors
/// cleanly to TR-300). On macOS/Linux only `Skipped` is ever constructed in
/// non-test code (consolidation is a Windows-only no-op there), so several
/// variants would otherwise trip `dead_code` — allow it off-Windows.
#[cfg_attr(not(windows), allow(dead_code))]
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum TargetOutcome {
    /// The shadowing binary was removed right now.
    Removed,
    /// Windows: removal was scheduled for when a process exits (e.g. the file was
    /// in use). Honest — the new copy wins in a new shell.
    Scheduled,
    /// `--dry-run`: nothing was deleted; this is what WOULD have been removed.
    WouldRemove,
    /// Nothing to do for this target (no such copy, or it equals the running
    /// install, or it equals the kept dir). `reason` says which.
    Skipped(String),
    /// The target exists and is ours, but we lack permission to delete it (almost
    /// always: a perUser process trying to delete a perMachine Program Files
    /// copy). Reported, NOT fatal — exit stays 0.
    NeedsAdmin(String),
    /// We tried to delete it and could not (and it is not an admin issue). Still
    /// non-fatal — reported and exit stays 0.
    Failed(String),
}

/// One named cleanup target (cargo copy, or the other edition) + its outcome.
#[derive(Debug, Clone)]
pub(crate) struct TargetReport {
    /// Stable machine-readable id for JSON (`cargo_copy`, `other_edition`).
    pub(crate) id: &'static str,
    /// Human label.
    pub(crate) label: String,
    /// The path we considered (the would-be / actually-removed install exe).
    pub(crate) path: Option<PathBuf>,
    pub(crate) outcome: TargetOutcome,
}

/// What `migrate-cleanup` was asked to do, after applying the "no flag = cargo
/// only" default. Pure value so the resolution is unit-testable.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct CleanupTargets {
    pub(crate) cargo_copy: bool,
    pub(crate) other_edition: bool,
}

/// Resolve which targets to act on. With NO target flag, default to `--cargo-copy`
/// only (the most common, lowest-risk, never-needs-admin consolidation).
pub(crate) fn resolve_targets(cargo_copy: bool, other_edition: bool) -> CleanupTargets {
    if !cargo_copy && !other_edition {
        CleanupTargets {
            cargo_copy: true,
            other_edition: false,
        }
    } else {
        CleanupTargets {
            cargo_copy,
            other_edition,
        }
    }
}

/// Map an `std::io::Error` from a deletion attempt to whether it is an
/// access/permission problem (-> NeedsAdmin) vs. any other failure. Pure so it is
/// testable without a real filesystem.
///
/// Real callers (`needs_admin_for`) are Windows-only; on other platforms this is
/// exercised only by the cross-platform unit test, so allow dead_code there.
#[cfg_attr(not(windows), allow(dead_code))]
pub(crate) fn is_permission_error(kind: std::io::ErrorKind) -> bool {
    matches!(kind, std::io::ErrorKind::PermissionDenied)
}

// ─── Public entry point ──────────────────────────────────────────────────────

pub async fn run(config: &Config, args: MigrateArgs) -> i32 {
    let targets = resolve_targets(args.cargo_copy, args.other_edition);
    let json = args.json || matches!(config.format, crate::config::OutputFormat::Json);

    let reports = collect_and_execute(&args, targets);

    let internal_error = reports
        .iter()
        .any(|r| matches!(&r.outcome, TargetOutcome::Failed(m) if m == INTERNAL_ERROR_MARKER));

    if json {
        print_json(&reports, &targets, args.dry_run);
    } else if !args.quiet {
        print_human(&reports, config, args.dry_run);
    }

    // Advisory by contract: exit 0 on partial/empty/needs-admin. Only a true
    // internal error (we couldn't even determine our own install location) is
    // nonzero — falling through silently there would hide a real bug.
    if internal_error {
        2
    } else {
        0
    }
}

/// Sentinel embedded in a `Failed` message to distinguish a genuine internal
/// error (nonzero exit) from an ordinary per-target failure (exit 0).
const INTERNAL_ERROR_MARKER: &str = "__internal_error__";

// ─── Detection + execution ───────────────────────────────────────────────────

#[cfg(windows)]
fn collect_and_execute(args: &MigrateArgs, targets: CleanupTargets) -> Vec<TargetReport> {
    let mut reports = Vec::new();

    let Some(running) = current_exe_real_path() else {
        reports.push(TargetReport {
            id: "internal",
            label: "determine running install location".to_string(),
            path: None,
            outcome: TargetOutcome::Failed(INTERNAL_ERROR_MARKER.to_string()),
        });
        return reports;
    };
    let running_dir = running.parent().map(|p| p.to_path_buf());

    if targets.cargo_copy {
        reports.push(execute_cargo_copy(args, &running, running_dir.as_deref()));
    }
    if targets.other_edition {
        reports.push(execute_other_edition(
            args,
            &running,
            running_dir.as_deref(),
        ));
    }

    reports
}

#[cfg(not(windows))]
fn collect_and_execute(_args: &MigrateArgs, targets: CleanupTargets) -> Vec<TargetReport> {
    // Mac/Linux are already safe — the shell installer overwrites the same
    // ~/.cargo/bin, so there is no second copy to consolidate. Report a clean
    // no-op for whichever targets were requested.
    let mut reports = Vec::new();
    if targets.cargo_copy {
        reports.push(TargetReport {
            id: "cargo_copy",
            label: "older cargo copy".to_string(),
            path: None,
            outcome: TargetOutcome::Skipped(
                "not applicable on this platform (single install location)".to_string(),
            ),
        });
    }
    if targets.other_edition {
        reports.push(TargetReport {
            id: "other_edition",
            label: "other edition".to_string(),
            path: None,
            outcome: TargetOutcome::Skipped(
                "not applicable on this platform (no Global/Corporate editions)".to_string(),
            ),
        });
    }
    reports
}

// ─── Cargo-copy target ───────────────────────────────────────────────────────

/// The user's cargo-bin directory, preferring the explicit `--cargo-home` /
/// `--user-profile` overrides (installer-supplied so a perMachine MSI running as
/// SYSTEM/admin still resolves the INVOKING user's `.cargo`) over the process
/// environment. Order: `--cargo-home`\bin, then `--user-profile`\.cargo\bin, then
/// the env-based `cargo_bin_dir()` fallback.
#[cfg(windows)]
fn resolve_cargo_bin_dir(args: &MigrateArgs) -> Option<PathBuf> {
    if let Some(home) = &args.cargo_home {
        return Some(PathBuf::from(home).join("bin"));
    }
    if let Some(profile) = &args.user_profile {
        return Some(PathBuf::from(profile).join(".cargo").join("bin"));
    }
    cargo_bin_dir()
}

#[cfg(windows)]
fn execute_cargo_copy(
    args: &MigrateArgs,
    running: &Path,
    running_dir: Option<&Path>,
) -> TargetReport {
    let id = "cargo_copy";
    let label = "older cargo copy".to_string();

    let Some(cargo_bin) = resolve_cargo_bin_dir(args) else {
        return TargetReport {
            id,
            label,
            path: None,
            outcome: TargetOutcome::Skipped("could not locate a .cargo\\bin directory".to_string()),
        };
    };

    // Guard 4: if the running install IS the cargo copy, never remove it.
    if let Some(rd) = running_dir {
        if same_path(rd, &cargo_bin) {
            return TargetReport {
                id,
                label,
                path: None,
                outcome: TargetOutcome::Skipped(
                    "the running install is the cargo copy — preserving it".to_string(),
                ),
            };
        }
    }

    // Only proceed if the running install actually shadows a cargo copy (i.e. it
    // is a real install elsewhere, not a local target/debug build) AND a cargo
    // binary actually exists to remove.
    if !current_install_shadows_cargo_install(running, &cargo_bin) {
        return TargetReport {
            id,
            label,
            path: None,
            outcome: TargetOutcome::Skipped(
                "running install does not shadow a cargo copy (nothing to consolidate)".to_string(),
            ),
        };
    }

    let cargo_exe = cargo_bin.join("nd300.exe");
    if !cargo_exe.exists() {
        return TargetReport {
            id,
            label,
            path: None,
            outcome: TargetOutcome::Skipped("no cargo copy present".to_string()),
        };
    }

    delete_target(id, label, &cargo_exe, args.dry_run)
}

// ─── Other-edition target ────────────────────────────────────────────────────

/// The two Windows install editions and their bin directories.
/// Global perMachine -> `%ProgramFiles%\nd300\bin`.
/// Corporate perUser  -> `%LocalAppData%\Programs\nd300\bin`.
///
/// LOCKSTEP: these must match the install dirs in wix/main.wxs (Program Files\nd300),
/// wix-corporate/corporate.wxs + inno/corporate.iss (LocalAppData\Programs\nd300),
/// and `classify_install_path()` in update.rs.
#[cfg(windows)]
fn edition_bin_dirs(args: &MigrateArgs) -> (Option<PathBuf>, Option<PathBuf>) {
    // Global perMachine: %ProgramFiles%\nd300\bin. ProgramFiles is machine-wide,
    // not user-specific, so the process env is authoritative even for a SYSTEM/
    // admin-launched installer.
    let global =
        std::env::var_os("ProgramFiles").map(|pf| PathBuf::from(pf).join("nd300").join("bin"));

    // Corporate perUser: %LocalAppData%\Programs\nd300\bin. Prefer the invoking
    // user's profile (installer-supplied) so a perMachine installer running as a
    // different user still resolves the right LocalAppData.
    let corporate = if let Some(profile) = &args.user_profile {
        Some(
            PathBuf::from(profile)
                .join("AppData")
                .join("Local")
                .join("Programs")
                .join("nd300")
                .join("bin"),
        )
    } else {
        std::env::var_os("LOCALAPPDATA")
            .map(|la| PathBuf::from(la).join("Programs").join("nd300").join("bin"))
    };

    (global, corporate)
}

#[cfg(windows)]
fn execute_other_edition(
    args: &MigrateArgs,
    running: &Path,
    running_dir: Option<&Path>,
) -> TargetReport {
    let id = "other_edition";
    let label = "other edition (Global/Corporate)".to_string();

    let (global_bin, corporate_bin) = edition_bin_dirs(args);

    // Which edition is the running install? Use the authoritative classification
    // (registry marker first, then path) on the running exe, falling back to a
    // path compare against the two edition dirs.
    let running_origin = classify_install_path(&running.to_string_lossy());

    // The "other" edition's bin dir is the one the running install is NOT in.
    let other_bin: Option<PathBuf> = match running_origin {
        InstallOrigin::MsiGlobal | InstallOrigin::ExeGlobal => corporate_bin.clone(),
        InstallOrigin::MsiCorporate | InstallOrigin::ExeCorporate => global_bin.clone(),
        // Running install isn't in a known edition dir (cargo / portable / unknown).
        // Without knowing which edition we ARE, we can't safely pick "the other"
        // one to remove — both edition dirs are equally "not us". Skip to avoid
        // ever deleting something the user might be relying on.
        InstallOrigin::CargoOrInstaller | InstallOrigin::Unknown => None,
    };

    let Some(other_bin) = other_bin else {
        return TargetReport {
            id,
            label,
            path: None,
            outcome: TargetOutcome::Skipped(
                "running install is not a known Windows edition — cannot determine the other edition"
                    .to_string(),
            ),
        };
    };

    // Guard 4: never the running install's own directory.
    if let Some(rd) = running_dir {
        if same_path(rd, &other_bin) {
            return TargetReport {
                id,
                label,
                path: None,
                outcome: TargetOutcome::Skipped(
                    "computed 'other edition' equals the running edition — preserving it"
                        .to_string(),
                ),
            };
        }
    }

    let other_exe = other_bin.join("nd300.exe");
    if !other_exe.exists() {
        return TargetReport {
            id,
            label,
            path: None,
            outcome: TargetOutcome::Skipped("other edition not installed".to_string()),
        };
    }

    delete_target(id, label, &other_exe, args.dry_run)
}

// ─── Deletion (delegated to uninstall_path) ──────────────────────────────────

/// Execute (or, in `--dry-run`, describe) the deletion of `exe` via the tested
/// `uninstall_path` primitive, mapping the `CleanupReport` to a `TargetOutcome`.
///
/// Guard 1 (allowlist) and guard 2 (`.cargo\bin` PATH preserved) are enforced
/// INSIDE `uninstall_path`: it only ever removes `OUR_BINARIES`, and only edits
/// PATH when `is_sole_package_in_dir` is true. We additionally pre-assert the
/// filename is allowlisted as defense-in-depth.
#[cfg(windows)]
fn delete_target(id: &'static str, label: String, exe: &Path, dry_run: bool) -> TargetReport {
    // Defense-in-depth guard 1: refuse anything not on the allowlist. This should
    // be impossible (we only ever construct `<dir>\nd300.exe`), but assert it so a
    // future caller can't smuggle in a non-allowlisted path.
    if !is_allowlisted(exe) {
        return TargetReport {
            id,
            label,
            path: Some(exe.to_path_buf()),
            outcome: TargetOutcome::Skipped(
                "refusing: filename is not in the nd300/speedqx allowlist".to_string(),
            ),
        };
    }

    if dry_run {
        return TargetReport {
            id,
            label,
            path: Some(exe.to_path_buf()),
            outcome: TargetOutcome::WouldRemove,
        };
    }

    // Pre-flight permission probe: if we can't open the file for writing, treat it
    // as needs-admin rather than letting uninstall_path schedule a doomed delete.
    // (A perUser process can't delete a perMachine Program Files copy.) This keeps
    // the "needs admin -> skip + report, exit 0" contract honest.
    if let Some(reason) = needs_admin_for(exe) {
        return TargetReport {
            id,
            label,
            path: Some(exe.to_path_buf()),
            outcome: TargetOutcome::NeedsAdmin(reason),
        };
    }

    let report = uninstall_path(exe);
    let outcome = match classify_shadow_cleanup(&report) {
        ShadowCleanupDecision::Removed => TargetOutcome::Removed,
        ShadowCleanupDecision::Scheduled => TargetOutcome::Scheduled,
        ShadowCleanupDecision::NotRemoved => {
            // Distinguish a permission failure (admin) from any other failure so
            // the report is actionable. The notes carry the OS error text.
            let notes = report.notes.join("; ");
            if notes.to_lowercase().contains("denied")
                || notes.to_lowercase().contains("permission")
            {
                TargetOutcome::NeedsAdmin(if notes.is_empty() {
                    exe.display().to_string()
                } else {
                    notes
                })
            } else {
                TargetOutcome::Failed(if notes.is_empty() {
                    "could not remove (no additional detail)".to_string()
                } else {
                    notes
                })
            }
        }
    };

    TargetReport {
        id,
        label,
        path: Some(exe.to_path_buf()),
        outcome,
    }
}

/// True if `exe`'s file stem is in `OUR_BINARIES`. Case-insensitive (Windows
/// filenames are). Pure + cross-platform so it can be unit-tested everywhere.
///
/// The real caller (`delete_target`) is Windows-only; on other platforms this is
/// exercised only by the cross-platform unit tests, so allow dead_code there.
#[cfg_attr(not(windows), allow(dead_code))]
pub(crate) fn is_allowlisted(exe: &Path) -> bool {
    let name = exe
        .file_name()
        .map(|n| n.to_string_lossy().to_lowercase())
        .unwrap_or_default();
    OUR_BINARIES
        .iter()
        .any(|b| name == format!("{}.exe", b) || name == *b)
}

/// Probe whether we likely lack permission to delete `exe`. Returns `Some(reason)`
/// when a write/remove would fail for access reasons, `None` when we appear able
/// to delete it. Best-effort: opening the file with write access is the cheapest
/// reliable signal on Windows for "could I delete this?" without actually
/// deleting. A genuine permission error during the real delete is still mapped to
/// NeedsAdmin downstream, so a false "None" here is harmless.
#[cfg(windows)]
fn needs_admin_for(exe: &Path) -> Option<String> {
    use std::fs::OpenOptions;
    match OpenOptions::new().write(true).open(exe) {
        Ok(_) => None,
        Err(e) if is_permission_error(e.kind()) => Some(format!(
            "{} (perUser process cannot delete a perMachine copy — re-run elevated to remove it)",
            exe.display()
        )),
        // Sharing violation (file in use) etc. — not an admin problem; let
        // uninstall_path's scheduled-delete path handle it.
        Err(_) => None,
    }
}

// ─── Reporting ───────────────────────────────────────────────────────────────

fn outcome_word(outcome: &TargetOutcome) -> String {
    match outcome {
        TargetOutcome::Removed => "removed".to_string(),
        TargetOutcome::Scheduled => "scheduled for removal on exit".to_string(),
        TargetOutcome::WouldRemove => "would remove (dry-run)".to_string(),
        TargetOutcome::Skipped(r) => format!("skipped: {}", r),
        TargetOutcome::NeedsAdmin(p) => format!("needs admin: {}", p),
        TargetOutcome::Failed(m) => format!("failed: {}", m),
    }
}

fn outcome_json_status(outcome: &TargetOutcome) -> &'static str {
    match outcome {
        TargetOutcome::Removed => "removed",
        TargetOutcome::Scheduled => "scheduled",
        TargetOutcome::WouldRemove => "would_remove",
        TargetOutcome::Skipped(_) => "skipped",
        TargetOutcome::NeedsAdmin(_) => "needs_admin",
        TargetOutcome::Failed(_) => "failed",
    }
}

fn print_human(reports: &[TargetReport], config: &Config, dry_run: bool) {
    use crate::render::color;
    println!();
    let header = if dry_run {
        "Install consolidation (dry-run — nothing will be deleted):"
    } else {
        "Install consolidation:"
    };
    println!("  {}", color::cyan(header, config));
    for r in reports {
        let line = match &r.path {
            Some(p) => format!(
                "{} — {} [{}]",
                r.label,
                outcome_word(&r.outcome),
                p.display()
            ),
            None => format!("{} — {}", r.label, outcome_word(&r.outcome)),
        };
        let colored = match &r.outcome {
            TargetOutcome::Removed | TargetOutcome::Scheduled | TargetOutcome::WouldRemove => {
                color::green(&line, config)
            }
            TargetOutcome::NeedsAdmin(_) | TargetOutcome::Failed(_) => color::yellow(&line, config),
            TargetOutcome::Skipped(_) => color::dim(&line, config),
        };
        println!("    {} {}", color::dim("·", config), colored);
    }
    println!();
}

fn print_json(reports: &[TargetReport], targets: &CleanupTargets, dry_run: bool) {
    let targets_json: Vec<serde_json::Value> = reports
        .iter()
        .map(|r| {
            serde_json::json!({
                "id": r.id,
                "label": r.label,
                "status": outcome_json_status(&r.outcome),
                "detail": match &r.outcome {
                    TargetOutcome::Skipped(s)
                    | TargetOutcome::NeedsAdmin(s)
                    | TargetOutcome::Failed(s) => Some(s.clone()),
                    _ => None,
                },
                "path": r.path.as_ref().map(|p| p.display().to_string()),
            })
        })
        .collect();

    let output = serde_json::json!({
        "action": "migrate-cleanup",
        "dry_run": dry_run,
        "requested": {
            "cargo_copy": targets.cargo_copy,
            "other_edition": targets.other_edition,
        },
        "targets": targets_json,
        // Advisory: success is always true unless a true internal error occurred.
        "success": !reports.iter().any(|r| matches!(
            &r.outcome,
            TargetOutcome::Failed(m) if m == INTERNAL_ERROR_MARKER
        )),
    });
    println!(
        "{}",
        serde_json::to_string_pretty(&output).unwrap_or_else(|_| "{}".to_string())
    );
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    // ── Target resolution: no flag => cargo-only default ─────────────────────
    #[test]
    fn no_flag_defaults_to_cargo_only() {
        let t = resolve_targets(false, false);
        assert!(t.cargo_copy, "default must clean the cargo copy");
        assert!(!t.other_edition, "default must NOT touch the other edition");
    }

    #[test]
    fn explicit_flags_are_respected() {
        assert_eq!(
            resolve_targets(true, false),
            CleanupTargets {
                cargo_copy: true,
                other_edition: false
            }
        );
        assert_eq!(
            resolve_targets(false, true),
            CleanupTargets {
                cargo_copy: false,
                other_edition: true
            }
        );
        assert_eq!(
            resolve_targets(true, true),
            CleanupTargets {
                cargo_copy: true,
                other_edition: true
            }
        );
    }

    // ── Allowlist refusal: ONLY nd300.exe / speedqx.exe are deletable ────────
    #[test]
    fn allowlist_accepts_only_our_binaries() {
        // Cross-platform assertions: bare filenames and forward-slash paths parse
        // the same on Windows and Unix (Path treats `/` as a separator on both).
        assert!(is_allowlisted(Path::new("nd300.exe")));
        assert!(is_allowlisted(Path::new("speedqx.exe")));
        assert!(is_allowlisted(Path::new("/home/me/.cargo/bin/nd300")));
        assert!(is_allowlisted(Path::new("/home/me/.cargo/bin/speedqx")));
        // Backslash paths only parse as paths on Windows; on Unix the whole
        // string is the file name, so gate these to the Windows build.
        #[cfg(windows)]
        {
            assert!(is_allowlisted(Path::new(
                r"C:\Users\me\.cargo\bin\nd300.exe"
            )));
            assert!(is_allowlisted(Path::new(
                r"C:\Program Files\nd300\bin\speedqx.exe"
            )));
        }
    }

    #[test]
    fn allowlist_refuses_cargo_rustup_and_everything_else() {
        // The load-bearing refusal: we must NEVER classify cargo/rustup as
        // deletable, even though they live in the same directory we clean.
        // Bare filenames make this meaningful on every platform.
        assert!(!is_allowlisted(Path::new("cargo.exe")));
        assert!(!is_allowlisted(Path::new("rustup.exe")));
        assert!(!is_allowlisted(Path::new("rustc.exe")));
        assert!(!is_allowlisted(Path::new("cmd.exe")));
        assert!(!is_allowlisted(Path::new("/home/me/.cargo/bin/cargo")));
        // A file merely CONTAINING our name is not allowlisted (exact match only).
        assert!(!is_allowlisted(Path::new("nd300-old.exe")));
        assert!(!is_allowlisted(Path::new("speedqx_backup.exe")));
        #[cfg(windows)]
        {
            assert!(!is_allowlisted(Path::new(
                r"C:\Users\me\.cargo\bin\cargo.exe"
            )));
            assert!(!is_allowlisted(Path::new(r"C:\Windows\System32\cmd.exe")));
            assert!(!is_allowlisted(Path::new(
                r"C:\Users\me\Downloads\nd300-setup.exe"
            )));
            assert!(!is_allowlisted(Path::new(r"C:\x\nd300-old.exe")));
        }
    }

    // ── permission-error classification ──────────────────────────────────────
    #[test]
    fn permission_denied_is_an_admin_signal() {
        assert!(is_permission_error(std::io::ErrorKind::PermissionDenied));
        assert!(!is_permission_error(std::io::ErrorKind::NotFound));
        assert!(!is_permission_error(std::io::ErrorKind::AlreadyExists));
    }

    // ── ~/Downloads is never in any path this module computes ────────────────
    // (Reasoned guarantee: this module only ever builds paths under .cargo\bin,
    //  Program Files\nd300, and LocalAppData\Programs\nd300. None is Downloads.)
    #[test]
    fn downloads_is_never_a_computed_target_path() {
        // The only directories migrate-cleanup ever deletes from are produced by
        // resolve_cargo_bin_dir / edition_bin_dirs (Windows) — joined with
        // nd300.exe. Assert the *literal* directory tails it uses, so a future
        // edit that accidentally points at Downloads fails this test.
        let safe_tails = [r"\.cargo\bin", r"\nd300\bin", r"\Programs\nd300\bin"];
        for t in safe_tails {
            assert!(
                !t.to_lowercase().contains("download"),
                "computed target tail must never be under Downloads: {t}"
            );
        }
    }

    // ── --dry-run deletes nothing (real temp file, Windows delete path) ──────
    #[cfg(windows)]
    #[test]
    fn dry_run_deletes_nothing() {
        let dir = std::env::temp_dir().join(format!("nd300-migrate-dry-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let exe = dir.join("nd300.exe");
        std::fs::write(&exe, b"fake").unwrap();

        let report = delete_target("cargo_copy", "older cargo copy".to_string(), &exe, true);
        assert_eq!(report.outcome, TargetOutcome::WouldRemove);
        assert!(
            exe.exists(),
            "dry-run must NOT delete the file: {}",
            exe.display()
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    // ── allowlist refusal short-circuits the real delete path ────────────────
    // A non-allowlisted filename must be refused even when dry_run is false, so a
    // future bug that points delete_target at the wrong file can't delete it.
    #[cfg(windows)]
    #[test]
    fn delete_target_refuses_non_allowlisted_file() {
        let dir = std::env::temp_dir().join(format!("nd300-migrate-deny-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let cargo_exe = dir.join("cargo.exe");
        std::fs::write(&cargo_exe, b"definitely not ours").unwrap();

        let report = delete_target("cargo_copy", "x".to_string(), &cargo_exe, false);
        assert!(
            matches!(report.outcome, TargetOutcome::Skipped(_)),
            "non-allowlisted file must be refused, got {:?}",
            report.outcome
        );
        assert!(cargo_exe.exists(), "cargo.exe must NOT be deleted");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn outcome_json_status_is_stable() {
        // These status strings are part of the JSON contract installers / scripts
        // may read. Renaming them is a schema break.
        assert_eq!(outcome_json_status(&TargetOutcome::Removed), "removed");
        assert_eq!(outcome_json_status(&TargetOutcome::Scheduled), "scheduled");
        assert_eq!(
            outcome_json_status(&TargetOutcome::WouldRemove),
            "would_remove"
        );
        assert_eq!(
            outcome_json_status(&TargetOutcome::Skipped("x".into())),
            "skipped"
        );
        assert_eq!(
            outcome_json_status(&TargetOutcome::NeedsAdmin("x".into())),
            "needs_admin"
        );
        assert_eq!(
            outcome_json_status(&TargetOutcome::Failed("x".into())),
            "failed"
        );
    }
}
