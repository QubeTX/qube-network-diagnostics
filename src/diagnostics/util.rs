//! Timeout wrappers for diagnostic subprocess calls and DNS resolution.
//!
//! Diagnostics frequently shell out (`ping`, `netsh`, `ipconfig`, `scutil`,
//! `nmcli`, …) or resolve hostnames. On a *broken* network — the exact
//! condition the tool exists to diagnose — those calls can block far longer
//! than is useful, or effectively hang (a black-holed resolver, a `ping`
//! waiting on a dead gateway, a subprocess stuck on a wedged service). This
//! module bounds every such call with `tokio::time::timeout` so a degraded
//! network surfaces a clean "unreachable/empty" result instead of a stall.
//!
//! The contract mirrors `actions/fix/cmd.rs::run_cmd`: take a
//! [`tokio::process::Command`] by value, race it against a timeout, and on
//! either a timeout *or* a spawn error return `None`. `None` is intentionally
//! byte-identical to the pre-existing `.ok()` == `None` / spawn-failure path
//! every caller already handles, so wrapping a call never changes behavior on
//! a healthy network — it only caps the worst case on a broken one.

use std::time::Duration;
use tokio::process::Command;

/// Budget for DNS resolution (`tokio::net::lookup_host`). Resolution should be
/// fast on a healthy network; a black-holed resolver is exactly what we cap.
pub const RESOLVE: Duration = Duration::from_secs(5);

/// Budget for fast, local subprocess calls that query OS state (registry,
/// `netsh` show, `ifconfig`, `netstat`, `scutil`, `nmcli`, `ip`, `arp`,
/// `route`, …). These don't touch the network and should return promptly.
pub const QUICK: Duration = Duration::from_secs(5);

/// Budget for subprocess calls that themselves perform network I/O and can
/// legitimately take several seconds (`ping`, `nslookup`, `dig`,
/// `resolvectl`). Wider than [`QUICK`] so we don't truncate a slow-but-working
/// probe.
pub const SLOW: Duration = Duration::from_secs(10);

/// Run a subprocess with a timeout.
///
/// Consumes `cmd`, races `cmd.output()` against `dur`, and returns:
/// - `Some(output)` if the process finished within `dur` — **regardless of its
///   own exit code** (callers inspect `output.status` exactly as before).
/// - `None` on timeout **or** spawn error (binary missing, OS-level failure).
///
/// The `None` case is identical to today's bare `cmd.output().await.ok()`
/// returning `None`, so existing fallback branches handle it unchanged.
pub async fn run_with_timeout(mut cmd: Command, dur: Duration) -> Option<std::process::Output> {
    match tokio::time::timeout(dur, cmd.output()).await {
        Ok(Ok(output)) => Some(output),
        // Spawn / run error (e.g. binary not found) — same as `.ok()` == None.
        Ok(Err(_)) => None,
        // Timed out — treat as an unreachable/empty result.
        Err(_) => None,
    }
}

/// Resolve a host:port string with a timeout.
///
/// Races `tokio::net::lookup_host(addr)` against `dur`. Returns:
/// - `Some(addrs)` with the collected [`SocketAddr`]s on success.
/// - `None` on resolver error **or** timeout.
///
/// The `None` case matches the resolver's pre-existing `Err(_)` arm, so callers
/// can map it onto their existing failure path verbatim.
///
/// [`SocketAddr`]: std::net::SocketAddr
pub async fn lookup_host_timeout(addr: String, dur: Duration) -> Option<Vec<std::net::SocketAddr>> {
    match tokio::time::timeout(dur, tokio::net::lookup_host(addr)).await {
        Ok(Ok(addrs)) => Some(addrs.collect()),
        // Resolver error — same as the bare `lookup_host(..).await` Err arm.
        Ok(Err(_)) => None,
        // Timed out (black-holed resolver) — treat as a resolution failure.
        Err(_) => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A command that completes well within budget returns `Some`.
    #[tokio::test]
    async fn fast_command_returns_some() {
        #[cfg(windows)]
        let mut cmd = {
            let mut c = Command::new("cmd");
            c.args(["/C", "exit", "0"]);
            c
        };
        #[cfg(unix)]
        let mut cmd = Command::new("true");

        // Suppress unused-mut on the unix branch where we don't push args.
        let _ = &mut cmd;

        let out = run_with_timeout(cmd, QUICK).await;
        assert!(out.is_some(), "fast command should finish within budget");
    }

    /// A command that sleeps longer than the (tiny) budget returns `None`.
    #[tokio::test]
    async fn slow_command_times_out_to_none() {
        #[cfg(windows)]
        let cmd = {
            // `timeout` is interactive/redirect-sensitive on Windows; use
            // `ping -n 3 127.0.0.1` which takes ~2s, well over our 1ms budget.
            let mut c = Command::new("cmd");
            c.args(["/C", "ping", "-n", "3", "127.0.0.1"]);
            c
        };
        #[cfg(unix)]
        let cmd = {
            let mut c = Command::new("sleep");
            c.arg("2");
            c
        };

        let out = run_with_timeout(cmd, Duration::from_millis(1)).await;
        assert!(out.is_none(), "command exceeding budget should yield None");
    }

    /// A missing binary returns `None` (spawn error), not a hang.
    #[tokio::test]
    async fn missing_binary_returns_none() {
        let cmd = Command::new("nd300-definitely-not-a-real-binary-xyz");
        let out = run_with_timeout(cmd, QUICK).await;
        assert!(out.is_none(), "missing binary should yield None");
    }

    /// A normal resolve of localhost succeeds within budget.
    #[tokio::test]
    async fn localhost_resolves_to_some() {
        let addrs = lookup_host_timeout("localhost:80".to_string(), RESOLVE).await;
        assert!(
            addrs.is_some_and(|v| !v.is_empty()),
            "localhost:80 should resolve to at least one address"
        );
    }

    // The resolver timeout-elapse → `None` path is covered deterministically by
    // `slow_command_times_out_to_none` above: both `run_with_timeout` and
    // `lookup_host_timeout` wrap the same `tokio::time::timeout`, so the
    // elapse → `None` branch is identical. A prior test raced a real
    // `lookup_host("example.com")` against a 1ns budget, which is
    // non-deterministic — `tokio::time::timeout` polls the inner future first, so
    // a fast/cached resolve can return `Some` before the timer fires (observed
    // flaking under concurrent test load). It was removed to keep the release
    // `cargo test` gate stable.
}
