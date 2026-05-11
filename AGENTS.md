# AGENTS.md

This file provides guidance to Codex (Codex.ai/code) when working with code in this repository.

## Project Overview

ND-300 (`nd300`) is a cross-platform CLI network diagnostics tool written in Rust. It runs 25+ diagnostic modules concurrently, supports user and technician modes, and includes a multi-stage network recovery system (`--fix`). Ships two binaries: `nd300` (main diagnostic) and `speedqx` (standalone speed test).

## Build & Run Commands

```bash
cargo build --release          # Release build → target/release/nd300 + speedqx
cargo build                    # Debug build (also generates man pages via build.rs)
cargo run -- [OPTIONS]         # Run nd300 with arguments
cargo run --bin speedqx -- [OPTIONS]  # Run speedqx with arguments
cargo run -- -t                # Technician mode (deep diagnostics)
cargo run -- --json            # JSON output
cargo run -- fix               # Run triage-loop fix (requires elevation; subcommand form, v3+)
cargo run -- -f                # Same as `cargo run -- fix` (legacy flag form, still works)
cargo run -- --fast            # Skip speed test
cargo run -- update            # Self-update to latest release (also `--update`)
cargo test --lib actions::fix::triage  # Triage planner unit tests
```

Tests: only the triage planner (`src/actions/fix/triage.rs`) currently has unit tests; the rest is validated by CI release builds across 6 platform targets.

## Architecture

### Core Flow

```
main.rs → Nd300Cli (clap parser, defined in src/cli.rs)
  → Subcommand dispatch (nd300 fix | update | clear-dns | uninstall) — preferred form
  → Falls through to legacy flag dispatch (--fix, --update, --clear-dns, --uninstall, -d/--dns)
  → Conditional exit: --dns (exits on failure, falls through to diagnostics on success)
  → config.rs (Config builder: colors, unicode, mode, format)
  → diagnostics/mod.rs (concurrent runner via tokio::join!)
    → 7 core diagnostics (concurrent)
    → Speed test (sequential — bandwidth-saturating)
    → 17 tech-mode diagnostics (concurrent, behind -t flag, uses SharedCache)
  → render/ (user_mode | tech_mode | json — selected by format + mode)
  → Exit code: 0=OK, 1=Warn, 2=Fail
```

`Nd300Cli` defines a `command: Option<Nd300Command>` field alongside the legacy boolean flags. If a subcommand is given, it takes precedence; otherwise the flag-form dispatch runs. Both forms produce identical behavior. Output flags (`--json`, `--ascii`, `--no-color`, `--verbose`) and `--yes` are marked `global = true` so they work in both positions.

### Module Layout

- **`src/cli.rs`** — Shared clap derive definitions (`Nd300Cli`, `SpeedQXCli`) used by both binaries and by `build.rs` for man page generation.
- **`src/actions/`** — Exit-early operations (`--fix`, `--clear-dns`, `--uninstall`, `--dns`, `--update`). The fix flow (`actions/fix/`) implements 3-stage graduated network recovery with VPN detection, connectivity checks between stages, Wi-Fi reconnection, and report generation (`report.rs` saves Markdown to `~/Downloads/`).
- **`src/diagnostics/`** — All diagnostic modules. Each is self-contained with platform-specific code via `#[cfg]` attributes. Each returns `(DiagnosticResult, Option<DetailStruct>)` — the detail struct carries rich data for rendering. `shared_cache.rs` pre-fetches subprocess outputs to deduplicate calls across tech-mode modules.
- **`src/speedtest/`** — Quad-provider speed test engine (Cloudflare + M-Lab NDT7 + LibreSpeed + fast.com). Provider clients: `cloudflare.rs`, `ndt7.rs`, `librespeed.rs`, `fastcom.rs`. `statistics.rs` implements the accuracy pipeline (trimean, IQR filter, slow-start discard, inverse-variance merge, bootstrap CI). `display.rs` handles speedqx-specific progress rendering. Both binaries share this module.
- **`src/render/`** — Output formatting. `table.rs` builds Unicode/ASCII box-drawing tables with ANSI-aware string functions (`visible_len`, `truncate_visible`). `color.rs` centralizes ANSI color output. `progress.rs` handles spinners.
- **`src/config.rs`** — Config builder with fluent API. The Config object threads through the entire render pipeline.
- **`src/error.rs`** — Unified `AppError` enum via `thiserror`.
- **`src/platform/`** — OS detection and elevation checks.

### Key Patterns

**Platform abstraction:** Compile-time `#[cfg(target_os)]` branching per module — no runtime OS checks. Windows uses `winapi` (GetIfEntry2, TCP/UDP tables), `ipconfig` crate, and WMI (tech-mode only). Unix uses `nix` and `libc`.

**Diagnostic module contract:** Every diagnostic module exports `pub async fn check() -> (DiagnosticResult, Option<DetailType>)`. DiagnosticResult carries status + summary; the optional detail struct carries rich data for rendering.

**Subprocess timeouts:** All subprocess calls in the fix flow use explicit timeouts (`TIMEOUT_QUICK` 15s, `TIMEOUT_MEDIUM` 30s, `TIMEOUT_SLOW` 60s) defined in `actions/fix/cmd.rs`.

**SharedCache (tech mode):** `diagnostics/shared_cache.rs` pre-fetches netstat, ipconfig, sysinfo::Networks, and default gateway data once, shared across 10+ tech-mode diagnostic modules. Built via `SharedCache::build_for_tech_mode()` before the tech diagnostic `tokio::join!`.

**DiagnosticStatus enum:** `Ok`, `Warn`, `Fail`, `Skip` — only the 8 core diagnostics contribute to exit code aggregation.

**Build script:** `build.rs` generates man pages (`man/nd300.1`, `man/speedqx.1`) at build time via `clap_mangen`. It uses a `#[path = "src/cli.rs"]` include with a stub `speedtest` module to compile the CLI structs in the build context.

### Dual-Binary Design

Both `nd300` and `speedqx` use the shared `speedtest` module. CLI definitions live in `src/cli.rs` (not in the binary files). The speedqx binary (`src/bin/speedqx.rs`) has its own progress display via `DisplayState` + `SpeedQXDisplay`, tracking phases (CfLatency → CfDownload → CfUpload → Ndt7Discovery → Ndt7Download → Ndt7Upload → Computing).

### Cross-Platform Build Targets

aarch64-apple-darwin, x86_64-apple-darwin, aarch64-unknown-linux-gnu, x86_64-unknown-linux-gnu, x86_64-unknown-linux-musl, x86_64-pc-windows-msvc

## Release Process

Releases are automated via `cargo-dist` v0.31.0 and crates.io publishing. Push a commit to `main` with a new `Cargo.toml` version to trigger `.github/workflows/release.yml`: it runs source checks, builds all 6 cargo-dist targets, creates the GitHub release / updater assets, uploads legacy installer aliases, and publishes the `nd300` crate to crates.io. Version tags still work as a manual release trigger, but the standard deploy path is now main-branch push. The WiX manifest (`wix/main.wxs`) must include components for both `nd300.exe` and `speedqx.exe`.

### Standard release workflow (run when shipping)

Triggers in user prompts: "release", "ship it", "tag the release", "push it out", "make this live", "go for it" (after change discussion).

1. **Bump version** in `Cargo.toml` (`[package] version`). Patch for fixes, minor for backward-compatible features, major for breaking changes (e.g. `--fix` semantics changed in v3.0.0). Every deploy to `main` must use a never-published version.
2. **Update `CHANGELOG.md`** with a new top entry in Keep-a-Changelog format. Date = today (use the host's current date, not training-data date).
3. **Update `README.md`** if the release adds user-visible flags / commands / behavior.
4. **Update `AGENTS.md`** (this file) if the architecture or workflow changed.
5. **Run local verification** before pushing: `cargo fmt --check`, `cargo test --lib`, `cargo test`, `cargo clippy --all-targets --all-features -- -D warnings`, `cargo publish --dry-run --locked --allow-dirty`, and `cargo package --list --locked --allow-dirty`. After committing, the same publish/package commands can be rerun without `--allow-dirty`.
6. If changing installer or release behavior, **run `cargo build --release`** locally on the active platform to catch obvious packaging/build failures. Note: this only catches errors on the active platform — see "macOS / Linux-only failures" below.
7. **Stage specific files** (`git add CHANGELOG.md Cargo.toml Cargo.lock src/... README.md` etc). Never `git add -A` / `git add .` — risks pulling in `nul`, `.env`, build artifacts, or temp files.
8. **Commit** with a descriptive `feat:` / `fix:` / `docs:` message and a `Co-Authored-By: Codex Opus 4.7 (1M context) <noreply@anthropic.com>` trailer.
9. **Push to main** with `git push origin main`. Do not create the release tag locally for the normal path; the workflow creates `vX.Y.Z` when it creates the GitHub release.
10. **Watch CI via Monitor — never foreground `gh run watch`.** A foreground watch burns 5+ minutes of context blocking on the run. Instead, pick up the run id (`gh run list --workflow=release.yml --limit 1`) and arm the `Monitor` tool with a poll loop that emits **one notification per job state-change** plus a terminal `RUN_DONE` line. You keep working (or hand back to the user) while CI runs; events arrive as chat notifications.
11. **Verify publish outputs** after CI succeeds: `git ls-remote --tags origin | grep vX.Y.Z`, `gh release view vX.Y.Z`, `cargo owner --list nd300`, and `cargo install nd300 --force`.

    Pattern (parameterize `RUN_ID`, `timeout_ms` ~1500000 for 25 min headroom, `persistent: false`):

    ```bash
    prev=""
    while true; do
      cur=$(gh run view RUN_ID --json jobs --jq '.jobs[]? | select(.status=="completed") | "\(.name): \(.conclusion)"' 2>/dev/null | sort)
      comm -13 <(echo "$prev") <(echo "$cur")
      prev=$cur
      status=$(gh run view RUN_ID --json status --jq '.status' 2>/dev/null)
      if [ "$status" = "completed" ]; then
        conclusion=$(gh run view RUN_ID --json conclusion --jq '.conclusion' 2>/dev/null)
        echo "RUN_DONE: $conclusion"
        break
      fi
      sleep 30
    done
    ```

    **Critical: use `gh --jq`, never pipe to `jq`.** `jq` is not on PATH in the Bash tool's shell on this Windows machine — piping through `jq` makes the script silently loop forever (errors go to stderr → output file, not stdout → events; status never parses, terminal check never fires). `gh` ships its own jq filter via the `--jq` flag, which works everywhere `gh` does.

    **After arming, verify liveness within 60s.** Read the Monitor's `.output` file once (path is in the start-monitor result). If it's empty or stderr-only, the script is broken — stop and re-arm. Don't trust silence: a broken Monitor and a working-but-quiet one look identical from the outside.

    Why this shape:
    - `comm -13 <(prev) <(cur)` against sorted lists emits only *new* completions per poll, so each finished job arrives as exactly one notification (not the cumulative list every 30s).
    - `gh run view --json` (not `gh pr checks`) works for tag-triggered runs that have no associated PR.
    - `status=="completed"` covers every terminal state (`success`, `failure`, `cancelled`, `skipped`, `timed_out`) — silence ≠ progress, so don't filter to `success` only.
    - 30s poll matches GitHub Actions API rate limits comfortably; total run is 5–7 minutes across all 6 targets, ~12 polls × 3 `gh` calls = under 40 API calls per release.
    - `RUN_DONE: <conclusion>` is your verdict marker — surface it to the user. If `conclusion` is `failure`, dive straight into `gh run view RUN_ID --log-failed` and start the patch-bump loop.

### Patch-bump loop (when CI fails)

A green local build does NOT prove a green CI build. The release CI builds on macOS aarch64, macOS x86_64, Linux gnu / musl, Linux aarch64, and Windows MSVC. A `#[cfg(target_os = "macos")]` block can compile fine on Windows because the cfg gates it out — and break only when CI runs the macOS target. **Always assume cross-platform CI is the source of truth.**

When the release CI fails:

1. **Pull the failing job's log**: `gh run view <run-id> --log-failed | head -200` or scope to a specific job: `gh run view --job <job-id> --log`. Filter for `error[E` or `error:` lines.
2. **Diagnose the root cause** — typically one of:
   - cfg-gated code calling a private function (E0603) that's only exposed on the active platform.
   - Unused-warning amplification on a target where conditional compilation drops a code path.
   - Missing platform-specific dependency or feature flag.
   - Toolchain / Rust version mismatch between local and CI.
3. **Fix the underlying issue**. Do not paper over with `#[allow]` unless the warning is genuinely platform-shape only.
4. **Bump the PATCH version** (e.g. `v3.0.0` → `v3.0.1`). Do not retag the same version — cargo-dist treats a tag as immutable, and re-tagging confuses GitHub Releases / installers / cached artifacts. Always go forward.
5. **Add a CHANGELOG entry** for the patch. One-line description of what was broken and that it's a build-only fix is fine — users don't need internal CI minutiae.
6. **Re-run steps 7–11** of the standard workflow. Verify locally first; commit; push to main; verify tag/release/crate after CI.
7. **Loop** if the patch run also fails. Bump again (`v3.0.2`, `v3.0.3`, ...) — never reuse a tag.

### Don't-do list (release safety)

- Never run `git push --force` or `--force-with-lease` to main during a release.
- Never re-tag an already-pushed version. Always go forward.
- Never `git add -A` — risks adding sensitive / unintended files.
- Never amend a commit that's already pushed (use a new commit).
- Never bypass `pre-commit` hooks (`--no-verify`) without explicit user authorization.
- Never delete a published tag or release artifact without explicit user authorization (downstream installers / package managers may already cache the URL).

## Key Dependencies

| Crate | Purpose |
|-------|---------|
| `clap` 4.5 | CLI parsing (derive) |
| `tokio` 1 (full) | Async runtime |
| `reqwest` 0.12 | HTTP (rustls-tls) |
| `sysinfo` 0.32 | System/network info |
| `default-net` 0.22 | Default gateway detection |
| `indicatif` 0.17 | Progress spinners |
| `owo-colors` 4 | ANSI colors |
| `crossterm` 0.28 | Terminal control |
| `tokio-tungstenite` 0.24 | WebSocket for NDT7 speed test |
| `winapi` 0.3 | Win32 API (Windows only) |
| `ipconfig` 0.3 | Windows adapter enumeration |
| `wmi` 0.14 | WMI queries (Windows tech-mode only) |
| `nix` 0.29 | Unix system calls |
| `clap_mangen` 0.2 | Man page generation (build-dep only) |

## Fix Flow — Diagnostic-Driven Triage Loop (v3.0+)

`nd300 fix` (and the legacy `nd300 -f` flag) is no longer a fixed Stage 1 → Stage 2 → Stage 3 sequence. It runs an iterative **triage loop** in `src/actions/fix/loop_runner.rs`:

```
1. Run baseline diagnostics (diagnostics::run_all)
2. If everything passes → exit 0 ("nothing to fix")
3. Detect hard-block patterns (no link / ISP outage / enterprise VPN) → exit with guidance
4. Compute actionable failures, group by root cause, build_plan() returns ordered Vec<Action>
5. For each action in plan:
     - High-risk actions: render structured RiskExplanation prompt; require explicit 'y'
     - Apply, capture ActionOutcome, sleep stabilization window
     - If fatal_environment_change → break out of plan, re-probe immediately
6. Re-run diagnostics. If now passing → done. Else continue to next iteration.
7. Bounded by: ≤6 iterations, ≤4-min wall clock, per-action max_attempts
```

### Module layout under `src/actions/fix/`

- **`action.rs`** — `Action`, `ActionId`, `Cost`, `Risk`, `Reversibility`, `RiskExplanation` types. The registry (`all_actions()`) returns every `Action` available on the current platform. `Action::apply` dispatches to a free function per `ActionId` via match. `Risk::High(RiskExplanation)` is a sealed enum variant: a high-risk Action *cannot* be constructed without a complete plain-language explanation.
- **`triage.rs`** — Pure planning. `actionable_failures()`, `group_by_root_cause()` (walks the hardcoded dependency DAG), `hard_block_detected()`, `build_plan()`. No IO. Unit-tested via `cargo test --lib actions::fix::triage`.
- **`session.rs`** — `Session` (per-run state: attempts, effectiveness, snapshots, action_log) + `Reporter` (plain-language output for stdout) + `FinalOutcome`. `WALL_CLOCK_CAP = 240s`.
- **`loop_runner.rs`** — `run_and_finalize(config) -> i32`: pre-flight elevation check, runs the loop, persists Markdown report, returns exit code. Handles JSON output via `print_json_outcome`.
- **`report.rs`** — Iteration-oriented Markdown report. Saves to `~/Downloads/nd300-fix-report-YYYYMMDD-HHMMSS.md`.
- **`stages.rs`** — Platform primitives (`disable_interface`, `enable_interface`, `restart_services`, `platform_stage3`, etc.) — kept and made `pub` so the action registry calls them directly. The legacy `run_stage1/2/3` orchestrators are still in this file but unused; safe to remove in a follow-up.
- **`cmd.rs`** — `run_cmd` (legacy, returns `Result<Output, String>`) + `run_cmd_capture` (new, returns structured `CmdOutcome` with stdout/stderr/exit/duration). Reports render `CmdOutcome` payloads.

### High-risk prompts

In interactive mode, every High-risk Action renders a multi-line block with `what`, `why`, side-effects, reversibility, and typical duration before running. Requires an **explicit `y`** — Enter alone, blank, or any non-`y` is treated as N. **`--yes` does NOT bypass these prompts** (by design — protects non-technicians from accidentally OK'ing destructive actions). In `--json` / non-interactive contexts, High-risk Actions are *skipped* (not auto-applied) with a clear marker in the report.

### What to do when adding a new fix primitive

1. Add an `ActionId` variant in `action.rs`.
2. Add a free `apply_*` function for it.
3. Add a match arm in `Action::apply`.
4. Append an `Action { ... }` entry in `all_actions()` declaring `targets`, `cost`, `risk`, `max_attempts`, `stabilization`. For `Risk::High`, attach a complete `RiskExplanation`.
5. Triage planning picks it up automatically — no changes needed in `triage.rs`.

Enterprise VPNs (Cisco, Zscaler, Palo Alto / GlobalProtect, F5, Check Point, Juniper) are never auto-disabled. Fix reports always go to `~/Downloads/nd300-fix-report-YYYYMMDD-HHMMSS.md`.

## Self-Update Implementation (`--update`)

**Reference implementation for other QubeTX tools.** This pattern can be copied to any Rust CLI that uses cargo-dist for releases.

### Architecture

The self-update feature lives in `src/actions/update.rs` and follows the same pattern as `--uninstall` (exit-early action, Config-aware, JSON support).

### Flow

```
1. GET https://api.github.com/repos/{OWNER}/{REPO}/releases/latest
   - Header: User-Agent: {binary}/{VERSION}
   - Header: Accept: application/vnd.github+json
   - Parse JSON → tag_name (e.g. "v2.9.0")
2. Strip 'v' prefix, compare semver numerically against current VERSION
3. If current >= latest → print "Already on latest" → exit 0
4. Detect installation method:
   - If binary path contains ".cargo" + "bin" → cargo install
   - Otherwise → cargo-dist installer (shell/PowerShell)
5. Execute platform-specific update:
   - Windows: powershell -ExecutionPolicy ByPass -c "irm {PS_INSTALLER_URL} | iex"
   - macOS/Linux: sh -c 'curl --proto "=https" --tlsv1.2 -LsSf {SHELL_INSTALLER_URL} | sh'
   - Cargo: cargo install nd300 --force
6. Report success/failure → exit code
```

### Key Constants (to customize per tool)

```rust
const RELEASES_URL: &str = "https://api.github.com/repos/{OWNER}/{REPO}/releases/latest";
const SHELL_INSTALLER: &str = "https://github.com/{OWNER}/{REPO}/releases/latest/download/nd300-installer.sh";
const PS_INSTALLER: &str = "https://github.com/{OWNER}/{REPO}/releases/latest/download/nd300-installer.ps1";
```

### Wiring

1. Add `--update` flag to clap struct(s) in `cli.rs`:
   ```rust
   #[arg(long = "update", help_heading = "Actions")]
   pub update: bool,
   ```
2. Add `pub mod update;` to `actions/mod.rs`
3. In `main.rs`, add exit-early handler (before `--fix`, after `--uninstall`):
   ```rust
   if cli.update {
       let exit_code = nd_300::actions::update::run(&config).await;
       std::process::exit(exit_code);
   }
   ```
4. For additional binaries (e.g. speedqx), add the same handler after CLI parsing

### Platform Notes

- **Windows**: The running `.exe` cannot be replaced in place. The PowerShell installer from cargo-dist handles this correctly (downloads to temp, replaces after process exit).
- **macOS/Linux**: Shell installer can replace the running binary since Unix allows unlinking running executables.
- **Cargo**: `cargo install --force` handles everything.
- **`#[cfg(not(windows))]`** on the `SHELL_INSTALLER` constant to suppress dead code warnings on Windows builds.

### JSON Output

Supports `--json` mode with structured output matching the uninstall pattern:
```json
{
  "action": "update",
  "success": true,
  "current_version": "2.8.0",
  "latest_version": "2.9.0",
  "update_available": true,
  "method": "installer"
}
```

## Speed Test Accuracy Pipeline

The statistics module (`src/speedtest/statistics.rs`) implements a multi-stage accuracy pipeline matching the [SpeedQX web speed test](https://speedqx.com/how-it-works):

1. **Collect raw samples**: Each HTTP request / WebSocket measurement yields an individual Mbps value
2. **Discard slow-start** (30%): First 30% of samples removed to eliminate TCP ramp-up
3. **Upload-specific**: Keep only fastest 50% of post-warmup samples (upload ramp-up is slower)
4. **IQR outlier filter**: Remove values outside Q1 - 1.5*IQR to Q3 + 1.5*IQR
5. **Modified trimean**: `(P10 + 8*P50 + P90) / 10` — Ookla-style, heavily median-weighted
6. **Winsorized cross-validation**: If Winsorized trimean diverges from IQR trimean by >15%, average both
7. **Inverse-variance merge**: Combine providers using `1/variance` weighting, clamped to [0.3, 0.7]
8. **Latency**: Fixed 0.4 CF / 0.6 NDT7 weights (NDT7's kernel MinRTT is structurally superior)
9. **Divergence detection**: Flag when providers differ by >30%
10. **Stability**: Coefficient of variation on combined samples (stable = CV < 0.15)
