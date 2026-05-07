use std::process::Output;
use std::time::{Duration, Instant};
use tokio::process::Command;

pub const TIMEOUT_QUICK: Duration = Duration::from_secs(15);
pub const TIMEOUT_MEDIUM: Duration = Duration::from_secs(30);
pub const TIMEOUT_SLOW: Duration = Duration::from_secs(60);

pub async fn run_cmd(mut cmd: Command, timeout: Duration) -> Result<Output, String> {
    let label = format!("{:?}", cmd.as_std().get_program());
    match tokio::time::timeout(timeout, cmd.output()).await {
        Ok(Ok(output)) => Ok(output),
        Ok(Err(e)) => Err(format!("{} failed: {}", label, e)),
        Err(_) => Err(format!("{} timed out after {}s", label, timeout.as_secs())),
    }
}

/// Structured record of a subprocess invocation. Carries enough information for
/// the fix-flow report to render full forensic detail (command, args, exit
/// code, captured streams, wall-time, and the failure mode if any).
#[derive(Debug, Clone)]
pub struct CmdOutcome {
    /// Program name (e.g. `"netsh"`, `"ipconfig"`).
    pub command: String,
    /// Arguments passed to the program, in order.
    pub args: Vec<String>,
    /// Process exit code if the program ran to completion. `None` indicates a
    /// spawn error or a timeout — distinguished by `error`.
    pub exit_code: Option<i32>,
    /// Captured stdout (lossy UTF-8 — best-effort, suitable for human display
    /// and Markdown reports). Trailing whitespace is preserved as-is.
    pub stdout: String,
    /// Captured stderr (same encoding caveat as `stdout`).
    pub stderr: String,
    /// Wall-clock duration from spawn through completion / timeout / spawn
    /// failure.
    pub duration: Duration,
    /// `true` when the program ran to completion AND returned exit code 0.
    /// `false` for any non-zero exit, spawn failure, or timeout.
    pub ok: bool,
    /// Populated when the spawn or run failed for a reason other than a
    /// non-zero exit code (e.g. binary not found, OS-level failure, timeout).
    pub error: Option<String>,
}

impl CmdOutcome {
    /// Returns a single human-readable line summarizing the outcome.
    /// Suitable for terminal status lines.
    pub fn summary(&self) -> String {
        if self.ok {
            format!("ok ({:.1}s)", self.duration.as_secs_f64())
        } else if let Some(err) = &self.error {
            format!("failed: {}", err)
        } else if let Some(code) = self.exit_code {
            format!("exit {} ({:.1}s)", code, self.duration.as_secs_f64())
        } else {
            "failed".to_string()
        }
    }

    /// Reconstruct an approximate command line for display in reports.
    pub fn cmdline(&self) -> String {
        let mut s = self.command.clone();
        for a in &self.args {
            s.push(' ');
            if a.contains(' ') {
                s.push('"');
                s.push_str(a);
                s.push('"');
            } else {
                s.push_str(a);
            }
        }
        s
    }
}

/// Capturing variant of [`run_cmd`]. Consumes the supplied [`Command`],
/// recording the program path and argv first so the resulting [`CmdOutcome`]
/// can be rendered in fix-flow reports without losing the invocation context.
///
/// Returns a [`CmdOutcome`] for every code path (success, non-zero exit, spawn
/// error, timeout) — callers don't need to handle a `Result`. Inspect
/// [`CmdOutcome::ok`] / [`CmdOutcome::error`] / [`CmdOutcome::exit_code`] to
/// branch on what happened.
pub async fn run_cmd_capture(mut cmd: Command, timeout: Duration) -> CmdOutcome {
    let std_cmd = cmd.as_std();
    let command = std_cmd.get_program().to_string_lossy().into_owned();
    let args: Vec<String> = std_cmd
        .get_args()
        .map(|s| s.to_string_lossy().into_owned())
        .collect();

    let started = Instant::now();
    let raced = tokio::time::timeout(timeout, cmd.output()).await;
    let duration = started.elapsed();

    match raced {
        Ok(Ok(output)) => {
            let exit_code = output.status.code();
            let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
            let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
            let ok = output.status.success();
            CmdOutcome {
                command,
                args,
                exit_code,
                stdout,
                stderr,
                duration,
                ok,
                error: None,
            }
        }
        Ok(Err(e)) => CmdOutcome {
            command,
            args,
            exit_code: None,
            stdout: String::new(),
            stderr: String::new(),
            duration,
            ok: false,
            error: Some(format!("spawn failed: {}", e)),
        },
        Err(_) => CmdOutcome {
            command,
            args,
            exit_code: None,
            stdout: String::new(),
            stderr: String::new(),
            duration,
            ok: false,
            error: Some(format!("timed out after {}s", timeout.as_secs())),
        },
    }
}
