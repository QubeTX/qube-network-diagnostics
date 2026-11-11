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

/// Serialize explicitly state-changing network commands and retain ownership of
/// their child process until it has exited or has been explicitly killed and
/// reaped.
///
/// A plain `timeout(command.output())` drops the future but not necessarily the
/// spawned process. For commands such as `networksetup ... off`, `ifconfig
/// ... down`, or DNS setters, that lets an expired child apply its mutation
/// *after* the restore path has run. This helper closes that race:
///
/// - one supervisor owns the child with `kill_on_drop(true)` as a final guard;
/// - timeout and caller cancellation explicitly kill and reap it;
/// - all callers serialize on one mutex, so a restore command cannot start
///   until an interrupted mutation's supervisor has finished reaping.
pub(crate) async fn run_owned_mutation(
    mut cmd: Command,
    timeout: Duration,
) -> Result<Output, String> {
    use tokio::sync::oneshot;

    static MUTATION_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

    let label = format!("{:?}", cmd.as_std().get_program());
    cmd.kill_on_drop(true);

    // The guard is borrowed from a static mutex and can therefore move into
    // the supervisor task. If this caller is cancelled, restoration blocks on
    // the same mutex until the supervisor has killed and reaped the old child.
    let mutation_guard = MUTATION_LOCK.lock().await;
    let (cancel_tx, cancel_rx) = oneshot::channel();
    let supervisor = tokio::spawn(async move {
        let _mutation_guard = mutation_guard;
        run_owned_mutation_child(cmd, timeout, cancel_rx).await
    });
    let mut cancel_guard = MutationCancelGuard {
        sender: Some(cancel_tx),
    };

    let result = supervisor
        .await
        .map_err(|error| format!("{} mutation supervisor failed: {}", label, error))?;
    cancel_guard.sender.take();
    result.map_err(|error| format!("{} {}", label, error))
}

/// Results from a paired state transition such as down/up, off/on, or
/// release/renew. The inverse is attempted even when the first command fails,
/// times out, or the caller is cancelled.
#[cfg(any(windows, target_os = "linux", test))]
pub(crate) struct MutationPairOutput {
    pub first: Result<Output, String>,
    pub inverse: Result<Output, String>,
}

/// Run a destructive command and its inverse under a detached, cancellation-
/// aware supervisor.
///
/// Individual commands use [`run_owned_mutation`], so a timed-out child is
/// killed and reaped before its inverse can acquire the mutation lock. The
/// sequence supervisor closes the second failure window: if the caller is
/// dropped after the first command succeeds (including during `dwell`), the
/// supervisor remains alive and runs the inverse before exiting.
#[cfg(any(windows, target_os = "linux", test))]
pub(crate) async fn run_owned_mutation_pair(
    first: Command,
    first_timeout: Duration,
    dwell: Duration,
    inverse: Command,
    inverse_timeout: Duration,
) -> Result<MutationPairOutput, String> {
    use tokio::sync::oneshot;

    static PAIR_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

    let (cancel_tx, cancel_rx) = oneshot::channel();
    let supervisor = tokio::spawn(async move {
        let _pair_guard = PAIR_LOCK.lock().await;
        run_owned_mutation_pair_child(
            first,
            first_timeout,
            dwell,
            inverse,
            inverse_timeout,
            cancel_rx,
        )
        .await
    });
    let mut cancel_guard = PairCancelGuard {
        sender: Some(cancel_tx),
    };

    let result = supervisor
        .await
        .map_err(|error| format!("mutation-pair supervisor failed: {error}"))?;
    cancel_guard.sender.take();
    Ok(result)
}

#[cfg(any(windows, target_os = "linux", test))]
struct PairCancelGuard {
    sender: Option<tokio::sync::oneshot::Sender<()>>,
}

#[cfg(any(windows, target_os = "linux", test))]
impl Drop for PairCancelGuard {
    fn drop(&mut self) {
        if let Some(sender) = self.sender.take() {
            let _ = sender.send(());
        }
    }
}

#[cfg(any(windows, target_os = "linux", test))]
async fn run_owned_mutation_pair_child(
    first: Command,
    first_timeout: Duration,
    dwell: Duration,
    inverse: Command,
    inverse_timeout: Duration,
    mut cancelled: tokio::sync::oneshot::Receiver<()>,
) -> MutationPairOutput {
    let first_result = {
        let first_run = run_owned_mutation(first, first_timeout);
        tokio::pin!(first_run);
        tokio::select! {
            biased;
            _ = &mut cancelled => None,
            result = &mut first_run => Some(result),
        }
    };

    let first_succeeded = first_result
        .as_ref()
        .is_some_and(|result| result.as_ref().is_ok_and(|output| output.status.success()));
    if first_succeeded && !dwell.is_zero() {
        tokio::select! {
            biased;
            _ = &mut cancelled => {}
            _ = tokio::time::sleep(dwell) => {}
        }
    }

    // Once cleanup begins it deliberately ignores further cancellation. Its
    // owned runner still bounds, kills, and reaps the inverse child.
    let inverse = run_owned_mutation(inverse, inverse_timeout).await;
    MutationPairOutput {
        first: first_result.unwrap_or_else(|| Err("was cancelled before completing".to_string())),
        inverse,
    }
}

/// macOS-named entrypoint used at every service/interface/DNS mutation site.
/// Keeping this wrapper explicit makes accidental fallback to `run_cmd` easy to
/// catch in source review while sharing the same owned runner with Windows and
/// Linux adapter bounces.
#[cfg(target_os = "macos")]
pub(crate) async fn run_macos_mutation(cmd: Command, timeout: Duration) -> Result<Output, String> {
    run_owned_mutation(cmd, timeout).await
}

struct MutationCancelGuard {
    sender: Option<tokio::sync::oneshot::Sender<()>>,
}

impl Drop for MutationCancelGuard {
    fn drop(&mut self) {
        if let Some(sender) = self.sender.take() {
            // A dropped receiver means the supervisor already completed; that
            // is harmless. Otherwise this wakes its kill-and-reap branch.
            let _ = sender.send(());
        }
    }
}

async fn run_owned_mutation_child(
    mut cmd: Command,
    timeout: Duration,
    mut cancelled: tokio::sync::oneshot::Receiver<()>,
) -> Result<Output, String> {
    use std::process::Stdio;
    use tokio::io::AsyncReadExt;

    cmd.stdout(Stdio::piped()).stderr(Stdio::piped());
    let mut child = cmd
        .spawn()
        .map_err(|error| format!("spawn failed: {error}"))?;
    let mut stdout = child
        .stdout
        .take()
        .ok_or_else(|| "could not capture stdout".to_string())?;
    let mut stderr = child
        .stderr
        .take()
        .ok_or_else(|| "could not capture stderr".to_string())?;

    enum Completion {
        Exited(Result<(std::process::ExitStatus, Vec<u8>, Vec<u8>), String>),
        TimedOut,
        Cancelled,
    }

    let completion = {
        let completed = async {
            let mut stdout_bytes = Vec::new();
            let mut stderr_bytes = Vec::new();
            let (status, stdout_result, stderr_result) = tokio::join!(
                child.wait(),
                stdout.read_to_end(&mut stdout_bytes),
                stderr.read_to_end(&mut stderr_bytes),
            );
            let status = status.map_err(|error| format!("wait failed: {error}"))?;
            stdout_result.map_err(|error| format!("stdout read failed: {error}"))?;
            stderr_result.map_err(|error| format!("stderr read failed: {error}"))?;
            Ok((status, stdout_bytes, stderr_bytes))
        };
        tokio::pin!(completed);
        tokio::select! {
            biased;
            _ = &mut cancelled => Completion::Cancelled,
            result = &mut completed => Completion::Exited(result),
            _ = tokio::time::sleep(timeout) => Completion::TimedOut,
        }
    };

    match completion {
        Completion::Exited(Ok((status, stdout, stderr))) => Ok(Output {
            status,
            stdout,
            stderr,
        }),
        Completion::Exited(Err(error)) => {
            terminate_and_reap_mutation(&mut child).await;
            Err(error)
        }
        Completion::TimedOut => {
            terminate_and_reap_mutation(&mut child).await;
            Err(format!("timed out after {}s", timeout.as_secs_f64()))
        }
        Completion::Cancelled => {
            terminate_and_reap_mutation(&mut child).await;
            Err("was cancelled and safely terminated".to_string())
        }
    }
}

async fn terminate_and_reap_mutation(child: &mut tokio::process::Child) {
    let _ = child.start_kill();
    let _ = tokio::time::timeout(Duration::from_secs(5), child.wait()).await;
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

#[cfg(test)]
mod portable_owned_mutation_tests {
    use super::*;
    use std::path::{Path, PathBuf};

    const HELPER_TEST: &str =
        "actions::fix::cmd::portable_owned_mutation_tests::mutation_child_helper";

    fn test_path(label: &str, suffix: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "nd300-portable-mutation-{label}-{}-{suffix}",
            std::process::id()
        ))
    }

    fn helper_command(
        state_file: &Path,
        started_file: &Path,
        delay_ms: u64,
        value: &str,
    ) -> Command {
        let mut command = Command::new(std::env::current_exe().expect("current test executable"));
        command.args(["--exact", HELPER_TEST, "--nocapture"]);
        command.env("ND300_MUTATION_HELPER", "1");
        command.env("ND300_MUTATION_HELPER_STATE", state_file);
        command.env("ND300_MUTATION_HELPER_STARTED", started_file);
        command.env("ND300_MUTATION_HELPER_DELAY_MS", delay_ms.to_string());
        command.env("ND300_MUTATION_HELPER_VALUE", value);
        command
    }

    async fn wait_for_file(path: &Path) {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(3);
        while !path.exists() && tokio::time::Instant::now() < deadline {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        assert!(path.exists(), "helper did not create {}", path.display());
    }

    async fn wait_for_contents(path: &Path, expected: &[u8]) {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(3);
        loop {
            if std::fs::read(path).is_ok_and(|contents| contents == expected) {
                return;
            }
            assert!(
                tokio::time::Instant::now() < deadline,
                "{} never reached expected state {:?}",
                path.display(),
                String::from_utf8_lossy(expected)
            );
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }

    fn cleanup(paths: &[&Path]) {
        for path in paths {
            let _ = std::fs::remove_file(path);
        }
    }

    #[test]
    fn mutation_child_helper() {
        if std::env::var_os("ND300_MUTATION_HELPER").is_none() {
            return;
        }
        let state = PathBuf::from(
            std::env::var_os("ND300_MUTATION_HELPER_STATE").expect("helper state path"),
        );
        let started = PathBuf::from(
            std::env::var_os("ND300_MUTATION_HELPER_STARTED").expect("helper started path"),
        );
        let delay_ms = std::env::var("ND300_MUTATION_HELPER_DELAY_MS")
            .expect("helper delay")
            .parse::<u64>()
            .expect("numeric helper delay");
        let value = std::env::var("ND300_MUTATION_HELPER_VALUE").expect("helper value");
        std::fs::write(started, b"started").expect("write helper started marker");
        std::thread::sleep(Duration::from_millis(delay_ms));
        std::fs::write(state, value).expect("write helper state");
    }

    #[tokio::test]
    async fn owned_timeout_prevents_late_mutation_on_every_platform() {
        let state = test_path("timeout", "state");
        let started = test_path("timeout", "started");
        cleanup(&[&state, &started]);
        std::fs::write(&state, b"up").unwrap();

        let error = run_owned_mutation(
            helper_command(&state, &started, 600, "down"),
            Duration::from_millis(50),
        )
        .await
        .unwrap_err();
        assert!(error.contains("timed out"), "unexpected error: {error}");
        // Do not require the nested test harness to reach its marker before
        // the deliberately tiny timeout. On a saturated CI host the owned
        // child can be spawned and safely killed before its test body is
        // scheduled; that is a valid timeout, not a failed precondition. The
        // handshaked cancellation test below covers a child known to be live.
        tokio::time::sleep(Duration::from_millis(800)).await;
        assert_eq!(std::fs::read(&state).unwrap(), b"up");
        cleanup(&[&state, &started]);
    }

    #[tokio::test]
    async fn completed_pair_reports_both_transitions() {
        let state = test_path("pair-complete", "state");
        let down_started = test_path("pair-complete", "down-started");
        let up_started = test_path("pair-complete", "up-started");
        cleanup(&[&state, &down_started, &up_started]);

        let pair = run_owned_mutation_pair(
            helper_command(&state, &down_started, 0, "down"),
            Duration::from_secs(5),
            Duration::ZERO,
            helper_command(&state, &up_started, 0, "up"),
            Duration::from_secs(5),
        )
        .await
        .unwrap();
        assert!(pair.first.unwrap().status.success());
        assert!(pair.inverse.unwrap().status.success());
        assert_eq!(std::fs::read(&state).unwrap(), b"up");
        cleanup(&[&state, &down_started, &up_started]);
    }

    #[tokio::test]
    async fn timed_out_pair_reaps_first_before_running_inverse() {
        let state = test_path("pair-timeout", "state");
        let down_started = test_path("pair-timeout", "down-started");
        let up_started = test_path("pair-timeout", "up-started");
        cleanup(&[&state, &down_started, &up_started]);
        std::fs::write(&state, b"initial").unwrap();

        let pair = run_owned_mutation_pair(
            helper_command(&state, &down_started, 600, "down"),
            Duration::from_millis(50),
            Duration::ZERO,
            helper_command(&state, &up_started, 0, "up"),
            Duration::from_secs(5),
        )
        .await
        .unwrap();

        assert!(
            pair.first.unwrap_err().contains("timed out"),
            "the first mutation should have timed out"
        );
        assert!(pair.inverse.unwrap().status.success());
        wait_for_contents(&state, b"up").await;
        tokio::time::sleep(Duration::from_millis(800)).await;
        assert_eq!(
            std::fs::read(&state).unwrap(),
            b"up",
            "timed-out first command mutated state after its inverse"
        );
        cleanup(&[&state, &down_started, &up_started]);
    }

    #[tokio::test]
    async fn cancelled_pair_runs_inverse_and_cannot_mutate_late() {
        let state = test_path("pair-cancel", "state");
        let down_started = test_path("pair-cancel", "down-started");
        let up_started = test_path("pair-cancel", "up-started");
        cleanup(&[&state, &down_started, &up_started]);
        std::fs::write(&state, b"initial").unwrap();

        let task = tokio::spawn(run_owned_mutation_pair(
            helper_command(&state, &down_started, 600, "down"),
            Duration::from_secs(5),
            Duration::from_secs(2),
            helper_command(&state, &up_started, 0, "up"),
            Duration::from_secs(5),
        ));
        wait_for_file(&down_started).await;
        task.abort();
        let _ = task.await;

        wait_for_file(&up_started).await;
        wait_for_contents(&state, b"up").await;
        tokio::time::sleep(Duration::from_millis(800)).await;
        assert_eq!(
            std::fs::read(&state).unwrap(),
            b"up",
            "cancelled first command mutated state after its inverse"
        );
        cleanup(&[&state, &down_started, &up_started]);
    }
}

#[cfg(all(test, unix))]
mod owned_mutation_tests {
    use super::*;
    use std::path::{Path, PathBuf};

    fn test_path(label: &str, suffix: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "nd300-owned-mutation-{label}-{}-{suffix}",
            std::process::id()
        ))
    }

    fn late_down_command(pid_file: &Path, started_file: &Path, state_file: &Path) -> Command {
        let mut command = Command::new("sh");
        command.args([
            "-c",
            "printf '%s' \"$$\" > \"$1\"; printf started > \"$2\"; sleep 1; printf down > \"$3\"",
            "nd300-owned-mutation-test",
            &pid_file.to_string_lossy(),
            &started_file.to_string_lossy(),
            &state_file.to_string_lossy(),
        ]);
        command
    }

    fn restore_up_command(state_file: &Path) -> Command {
        let mut command = Command::new("sh");
        command.args([
            "-c",
            "printf up > \"$1\"",
            "nd300-owned-mutation-restore",
            &state_file.to_string_lossy(),
        ]);
        command
    }

    async fn wait_for_file(path: &Path) {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
        while !path.exists() && tokio::time::Instant::now() < deadline {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        assert!(
            path.exists(),
            "test child did not create {}",
            path.display()
        );
    }

    fn child_is_gone(pid_file: &Path) -> bool {
        let pid = std::fs::read_to_string(pid_file)
            .ok()
            .and_then(|value| value.parse::<libc::pid_t>().ok())
            .expect("test child PID");
        // SAFETY: signal zero performs existence/permission checking only and
        // has no pointer arguments.
        (unsafe { libc::kill(pid, 0) }) != 0
    }

    fn cleanup(paths: &[&Path]) {
        for path in paths {
            let _ = std::fs::remove_file(path);
        }
    }

    #[tokio::test]
    async fn timed_out_down_is_killed_reaped_before_restore_returns() {
        let pid_file = test_path("timeout", "pid");
        let started_file = test_path("timeout", "started");
        let state_file = test_path("timeout", "state");
        cleanup(&[&pid_file, &started_file, &state_file]);
        std::fs::write(&state_file, b"up").unwrap();

        let error = run_owned_mutation(
            late_down_command(&pid_file, &started_file, &state_file),
            Duration::from_millis(50),
        )
        .await
        .unwrap_err();
        assert!(error.contains("timed out"), "unexpected error: {error}");
        assert!(started_file.exists());
        assert!(
            child_is_gone(&pid_file),
            "timed-out mutation child was not reaped before return"
        );

        run_owned_mutation(restore_up_command(&state_file), Duration::from_secs(2))
            .await
            .unwrap();
        tokio::time::sleep(Duration::from_millis(1200)).await;
        assert_eq!(std::fs::read(&state_file).unwrap(), b"up");
        cleanup(&[&pid_file, &started_file, &state_file]);
    }

    #[tokio::test]
    async fn cancelled_down_supervisor_finishes_before_serialized_restore() {
        let pid_file = test_path("cancel", "pid");
        let started_file = test_path("cancel", "started");
        let state_file = test_path("cancel", "state");
        cleanup(&[&pid_file, &started_file, &state_file]);
        std::fs::write(&state_file, b"up").unwrap();

        let task = tokio::spawn(run_owned_mutation(
            late_down_command(&pid_file, &started_file, &state_file),
            Duration::from_secs(10),
        ));
        wait_for_file(&started_file).await;
        task.abort();
        let _ = task.await;

        // This uses the same mutation lock. It cannot run until cancellation
        // has killed and reaped the old `down` child.
        run_owned_mutation(restore_up_command(&state_file), Duration::from_secs(2))
            .await
            .unwrap();
        assert!(
            child_is_gone(&pid_file),
            "cancelled mutation child was not reaped before restore"
        );
        tokio::time::sleep(Duration::from_millis(1200)).await;
        assert_eq!(
            std::fs::read(&state_file).unwrap(),
            b"up",
            "late cancelled mutation overwrote restored state"
        );
        cleanup(&[&pid_file, &started_file, &state_file]);
    }
}
