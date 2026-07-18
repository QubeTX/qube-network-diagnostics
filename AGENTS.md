# AGENTS.md

This file provides guidance to Codex (Codex.ai/code) when working with code in this repository.

## Project Overview

ND-300 (`nd300`) is a cross-platform CLI network diagnostics tool written in Rust. It runs 25+ diagnostic modules concurrently, supports user and technician modes, and includes a diagnostic-driven network recovery system (`nd300 fix` / `--fix`). Ships two binaries: `nd300` (main diagnostic) and `speedqx` (standalone speed test).

For the v3.6.0 release handoff and later hardening, preserve each task's
rationale and impact when updating its status, and attach exact-SHA/run evidence
before marking a release gate done.

## Task management system

This repo uses the SHAUGHV `tasks-*` system. The board source of truth is
`.tasks/TASKS.md`; milestones live in `.tasks/MILESTONES.md`, and each task's
rich handoff lives at `.tasks/tasks/<id>.md` with its `## Verification`,
`## Status`, and attributed `## Activity` entries kept current while work is
in flight.

Use indented checkbox subtasks for small required steps that must be visible
and complete before their parent. Put reasoning, implementation context,
impact, acceptance, and resume notes in the parent detail file. Work large
enough to need its own status or owner is a top-level task linked with
`(needs #id)`. Never complete a task over unchecked subtasks or open
verification items; verify them or waive them with a dated reason. Never close
a milestone over open child tasks.

Never put secrets in task, detail, working-memory, or deep-memory files. Use
environment variables, the OS keychain, or `.tasks/secure/` (gitignored). This
board is shared through git: pull before board sessions, respect `(owner
name)`, attribute Activity lines, and commit meaningful board changes.

The live board port is per-repo. Resolve it from
`.tasks/.board-server.json` or `node .tasks/board-server.mjs status` and verify
the reported root; never assume port 4317. Relevant skills are `tasks-start`,
`tasks-create`, `tasks-management`, `tasks-update`, `tasks-memory`,
`tasks-boards`, and `tasks-remove`.

## Build & Run Commands

```bash
cargo build --release          # Release build → target/release/nd300 + speedqx
cargo build                    # Debug build (also generates man pages via build.rs)
cargo run --bin nd300 -- [OPTIONS]    # Run nd300 with arguments
cargo run --bin speedqx -- [OPTIONS]  # Run speedqx with arguments
cargo run --bin nd300 -- -t           # Technician mode (deep diagnostics)
cargo run --bin nd300 -- --json       # JSON output
cargo run --bin nd300 -- fix          # Run triage-loop fix (requires elevation; subcommand form, v3+)
cargo run --bin nd300 -- -f           # Same as `cargo run --bin nd300 -- fix` (legacy flag form, still works)
cargo run --bin nd300 -- --fast       # Skip speed test
cargo run --bin nd300 -- fix --fast   # Skip speed test inside the fix subcommand
cargo run --bin nd300 -- update       # Self-update to latest release (also `--update`)
cargo test --lib actions::fix::triage  # Triage planner unit tests
```

Security advisories: `cargo audit` (install once via `cargo install cargo-audit --locked`) auto-reads the project's `.cargo/audit.toml` ignore-list — when an advisory can't be cleared by a dep bump, add it there with a rationale instead of re-triaging. On a Windows host, use `cargo tree --target all -i <crate>` to see platform-gated transitive deps (e.g. the Linux netlink stack that pulls `paste`); plain `cargo tree` only shows host-target deps.

Tests cover the fix planner and confirmation rules, platform command construction, latency argument selection, speed-test aggregation, and updater Cargo-install migration behavior. CI release builds remain the source of truth for all 6 platform targets.

## Architecture

### Core Flow

```
main.rs → Nd300Cli (clap parser, defined in src/cli.rs)
  → Subcommand dispatch (nd300 dns | fix | update | clear-dns | uninstall | migrate-cleanup[hidden]) — preferred form
  → Falls through to legacy flag dispatch (--fix, --update, --clear-dns, --uninstall, -d/--dns)
  → Conditional exit: nd300 dns / --dns (exits on failure, falls through to diagnostics on success)
  → config.rs (Config builder: colors, unicode, mode, format)
  → diagnostics/mod.rs (concurrent runner; run_all(config, cap) enforces the wall-clock cap internally)
    → 7 core diagnostics (spawned as JoinHandles, raced against the cap) + speed test (sequential) = 8 core total
    → On cap expiry: completed checks are real, unfinished ones become fabricated `timed_out` Fail rows — partial results always render
    → tech mode (-t): T1 = 21 deep (concurrent, boxed futures) → T2 = route_path ∥ packet_loss ∥ path_mtu → T2b = nat + arp_health (derived) → T3 = bufferbloat (sequential, saturating) = 25 total, uses SharedCache
  → render/ (user_mode | tech_mode | json — selected by format + mode)
  → Exit code: 0=OK, 1=Warn, 2=Fail
```

Diagnostic counts: README/CHANGELOG cite the canonical totals (**8 core**, **25 deep**). The flow box splits each total into its concurrent groups plus the sequential outliers (speed test for core; bufferbloat for tech) — same modules, just grouped by how they run. Keep these reconciled when adding a module. Tech-mode module futures are **boxed** inside the `tokio::join!` — a flat 21-way join of inlined async state machines overflowed the Windows main-thread stack.

`Nd300Cli` defines a `command: Option<Nd300Command>` field alongside the legacy boolean flags. If a subcommand is given, it takes precedence; otherwise the flag-form dispatch runs. Both forms produce identical behavior. Output flags (`--json`, `--ascii`, `--no-color`, `--verbose`), fix confirmation (`--yes`), and speed controls (`--fast`, `--speed-duration`) are marked `global = true` so they work in both positions.

### Module Layout

- **`src/cli.rs`** — Shared clap derive definitions (`Nd300Cli`, `SpeedQXCli`) used by both binaries and by `build.rs` for man page generation.
- **`src/actions/`** — Exit-early operations (`fix`, `clear-dns`, `uninstall`, `dns`, `update` — each available as a bare subcommand and a legacy flag; plus hidden installer helpers). `dns` is *semi*-exit-early (exit on failure, fall through to diagnostics on success). The fix flow (`actions/fix/`) implements evidence-driven recovery, LIFO restoration, exact macOS DNS/search rollback, and private invoking-user reports. The updater (`actions/update.rs` + Unix-only `unix_install.rs`) proves one installation channel and never cross-falls-back: Cargo stays Cargo, a receipt-owned Unix archive stays archive-managed, a hash/receipt-proven macOS PKG downloads and opens its exact direct PKG, Windows standalone stays standalone, and each registered Windows MSI/EXE edition stays exact. `takeover.rs` is the hidden same-scope Windows fresh-install helper; `migrate.rs` retains narrow installer-authorized Cargo/edition cleanup and the retired-image allowlist; `maintenance.rs` owns MSI commit-time marker and legacy Global PATH maintenance. Registered Windows uninstall delegates directly to a Windows-Installer-proven MSI product code or validated Inno uninstaller; a macOS PKG uninstall removes only its proven pair/metadata and forgets its receipt; unknown/package-manager-owned Unix paths are refused. See "Self-Update Implementation" and installer sections below.
- **`src/actions/migrate.rs`** — Cross-method install cleanup, exposed as the **hidden** `nd300 migrate-cleanup` subcommand (`#[command(hide = true)]`). The four Windows installers use the Windows origins plus `--retired-update`; the Apple PKG postinstall uses `--install-origin macos-pkg --cargo-copy` as the console user. Windows cleanup retains its synchronous/locked-pair behavior and strict retired-image allowlist. macOS cleanup accepts only a Cargo registry or exact cargo-dist receipt in the resolved user's standard Cargo home, invokes that owning uninstaller, and retains ambiguous files. Publicly usable flags remain `--cargo-copy`, `--other-edition`, `--quiet`, `--dry-run`, `--json`, `--user-profile <path>`, `--cargo-home <path>`; installer origins are hidden. Hard guarantees remain unit-tested: never cargo/rustup/PATH/unknown files/the running package pair/`~/Downloads`; never escalates. **Always exits 0** except on a true internal error — cleanup is advisory and must never fail an active installer transaction.
- **`src/diagnostics/`** — All diagnostic modules. Core modules export `pub async fn check() -> (DiagnosticResult, Option<DetailStruct>)`; deep (tech-mode) modules export `pub async fn collect() -> Option<DetailType>` (Skip = `None`, section omitted). Platform parsers are pure fns gated `#[cfg(any(target_os = "...", test))]` so they unit-test on all three CI OSes. `ping.rs` is the single home for ping invocation/parsing. `shared_cache.rs` pre-fetches subprocess outputs to deduplicate calls across tech-mode modules. On macOS, `interfaces.rs` builds the topology truth from route/flags/hardware-service mapping/addresses; `wifi.rs` uses bounded `system_profiler` only in technician mode; VPN, IPv6, TLS/DNSSEC, routing/listener/DHCP/protocol parsers use captured modern/legacy fixtures. `util.rs` provides timeout wrappers whose timed-out children are terminated, plus `retry_probe`/`ping_budget`/`harvest_or`. Core verdicts require *consistent* failure (multi-burst gateway, 3-domain median DNS, dual-endpoint ports, ≥2-reachable latency) — see CLAUDE.md "Verdict stability".
- **`src/speedtest/`** — Shared Methodology v4 engine. Standalone speedqx has eight sources: Cloudflare, M-Lab NDT7, LibreSpeed, fast.com, M-Lab MSAK, Apple networkQuality, CacheFly, and Vultr; ND300 core stays Cloudflare + NDT7. Provider clients live beside `stat_primitives.rs`, `statistics.rs`, golden parity tests, adaptive transfer sizing, and display code. Ookla remains excluded for EULA reasons documented in `applenq.rs`.
- **`src/render/`** — Output formatting. `table.rs` builds Unicode/ASCII box-drawing tables with ANSI-aware string functions (`visible_len`, `truncate_visible`). `color.rs` centralizes ANSI color output. `progress.rs` handles spinners.
- **`src/config.rs`** — Config builder with fluent API. The Config object threads through the entire render pipeline.
- **`src/error.rs`** — Unified `AppError` enum via `thiserror`.
- **`src/platform/`** — OS detection/elevation plus Unix `invoking_user.rs`, which validates numeric `SUDO_UID`/passwd ownership and supplies the original user's home, Cargo paths, and privilege-dropping command setup.

### Key Patterns

**Platform abstraction:** Compile-time `#[cfg(target_os)]` branching per module — no runtime OS checks. Windows uses `winapi` (GetIfEntry2, TCP/UDP tables), `ipconfig` crate, and WMI (tech-mode only). Unix uses `nix` and `libc`.

**Diagnostic module contract:** Every diagnostic module exports `pub async fn check() -> (DiagnosticResult, Option<DetailType>)`. DiagnosticResult carries status + summary; the optional detail struct carries rich data for rendering.

**Subprocess timeouts:** All fix commands use explicit timeout classes from `actions/fix/cmd.rs`. Diagnostic subprocess/DNS calls are bounded by `diagnostics/util.rs`; a timed-out child is killed and reaped rather than left running. The Apple provider adds progress heartbeats, a whole-phase deadline, and capped exponential backoff with jitter. `run_all` returns partial results at its cap and `main.rs` retains the Ctrl-C race.

**SharedCache (tech mode):** `diagnostics/shared_cache.rs` pre-fetches netstat, ipconfig, sysinfo::Networks, and default gateway data once, shared across 10+ tech-mode diagnostic modules. Built via `SharedCache::build_for_tech_mode()` before the tech diagnostic `tokio::join!`.

**DiagnosticStatus enum:** `Ok`, `Warn`, `Fail`, `Skip` — only the 8 core diagnostics (the 7 concurrent + speed test) contribute to exit code aggregation; tech-mode deep diagnostics do not.

**Build script:** `build.rs` generates man pages (`man/nd300.1`, `man/speedqx.1`) at build time via `clap_mangen`. It uses a `#[path = "src/cli.rs"]` include with a stub `speedtest` module to compile the CLI structs in the build context.

### Dual-Binary Design

Both `nd300` and `speedqx` use the shared `speedtest` module. CLI definitions live in `src/cli.rs` (not in the binary files). The speedqx binary (`src/bin/speedqx.rs`) has its own progress display via `DisplayState` + `SpeedQXDisplay`, tracking phases (CfLatency → CfDownload → CfUpload → Ndt7Discovery → Ndt7Download → Ndt7Upload → Computing).

### Cross-Platform Build Targets

aarch64-apple-darwin, x86_64-apple-darwin, aarch64-unknown-linux-gnu, x86_64-unknown-linux-gnu, x86_64-unknown-linux-musl, x86_64-pc-windows-msvc

## Release Process

Releases use **cargo-dist v0.31.0, tag-triggered**, plus CI-gated crates.io publish. Five workflows: **`ci.yml`** (cross-platform code/audit/dist gates), **`crates-publish.yml`** (the only crate publisher), **`release.yml`** (six targets, Global MSI, base GitHub Release), **`windows-installers.yml`** (Corporate MSI + two Inno EXEs and sidecars → 28 assets), and **`macos-installer.yml`** (direct native universal Developer ID Installer PKG plus a legacy-compatible Developer ID Application DMG and both sidecars → **32 assets**). The macOS workflow has an internal-PR candidate path that builds/signs/notarizes and runs the direct-package lifecycle on native Intel and Apple Silicon before merge; it also proves the DMG contains byte-identical PKG bytes for immutable older updaters. After tag publication, both native hosts install immutable v3.7.2 and exercise the compatibility package transition before the reusable workflow can complete. Tag publication reuses the validated tag/SHA context, starts from the exact signed architecture archives, refuses asset overwrite, and separately attests all four additions. Release announcement requires all platform workflows, both compatibility-bridge rows, and exact 32-asset verification. Workflow defaults stay `contents: read`; only hosting gets write/id-token permission. **A deploy is two pushes: merge to `main` (→ crate), then push immutable `vX.Y.Z` (→ binaries/installers/release).**

Mac release credentials are the repository secrets `APPLE_CERTIFICATE_P12_BASE64`, `APPLE_CERTIFICATE_PASSWORD`, `APPLE_INSTALLER_CERTIFICATE_P12_BASE64`, `APPLE_INSTALLER_CERTIFICATE_PASSWORD`, `APPLE_API_KEY_P8_BASE64`, `APPLE_API_KEY_ID`, and `APPLE_API_ISSUER_ID`, plus repository variables `APPLE_SIGNING_IDENTITY=739B04530883FF9B665C66BD464F98C622971B32`, `APPLE_INSTALLER_SIGNING_IDENTITY=Developer ID Installer: ES Development LLC (M9D5379H93)`, and `APPLE_TEAM_ID=M9D5379H93`. Never print or commit secret values. The Installer certificate must be G2/RSA-2048 with the matching private key; both native candidate runners prove it by signing a disposable PKG before packaging proceeds.

### Standard release workflow (run when shipping) — tag-push, consistent with TR-300

Triggers in user prompts: "release", "ship it", "deploy", "tag the release", "push it out", "make this live", "cut a release", "go for it" (after change discussion).

1. **Bump the version in `Cargo.toml`** (`[package] version`) — patch/minor/major; always forward (never-published). This ONE number drives both the crate (crates-publish.yml on the `main` merge) and the GitHub/installer release (the `vX.Y.Z` tag).
2. **Update `CHANGELOG.md` + `HUMAN_CHANGELOG.md`** in lockstep (every technical entry needs a plain-English counterpart). Date = the host's current date.
3. **Update `README.md`** (user-visible flags/commands/behavior) and **`CLAUDE.md` / `AGENTS.md`** (architecture/workflow changes).
4. **Bump the homepage version fallbacks** in `qube-machine-report-homepage` — `useGitHubVersion('QubeTX/qube-network-diagnostics', '<new>')` in `ND300Install.jsx`, `ND300Hero.jsx`, `ND300Footer.jsx`, `InstallGuideContent.jsx`. Own PR → merge to `main` → Vercel auto-deploys.
5. **Local verification:** `cargo fmt --all -- --check`, Rust 1.97 `cargo clippy --locked --workspace --all-targets -- -D warnings`, `cargo test --locked --workspace --all-targets`, `cargo build --release --locked`, `cargo package --locked --list`, `cargo publish --dry-run --locked`, `cargo audit`, `dist plan`, `actionlint .github/workflows/*.yml`, ShellCheck release scripts, archive/man inspection, updater fault tests, and read-only native Mac smokes. A one-time user-authorized active-Mac happy path passed under an independent root watchdog during v3.6.0 hardening, but it is supplementary only. Future service-cycle live tests require an offline watchdog on a disposable VM/service; never use an active control-dependent network.
6. **Stage specific files** (never `git add -A`). **Commit** (`feat:`/`fix:`/`docs:`) with trailer `Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>`. Open a PR; it runs `ci.yml` on macOS + Linux + Windows.
7. **Merge the PR to `main`.** `ci.yml` → `crates-publish.yml` publishes the `nd300` crate (idempotent; a docs-only/unchanged-version merge safely skips).
8. **Push the release tag:** `git fetch origin main && git tag vX.Y.Z origin/main && git push origin vX.Y.Z` → fires `release.yml` → chains `windows-installers.yml`. **Never reuse a tag** (forward only).
9. **Watch + verify:** `release.yml`, its reusable Windows job, then its reusable macOS package job (poll-loop below). Confirm the crate published, the release has **exactly 32 assets**, direct PKG and compatibility DMG Apple tickets/public Gatekeeper checks pass, their package bytes agree, every attestation binds to the tag SHA, and `cargo install nd300 --force` works.

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

A green local build does NOT prove a green CI build. **`ci.yml` now compiles + tests on macOS + Linux + Windows at PR time** (`-D warnings`), catching most cfg/dead-code/per-OS issues before merge — but the WiX/Inno **installer builds** only run in the tag-triggered `release.yml` + `windows-installers.yml`, and the full 6-target matrix (Linux musl, Linux/macOS aarch64) only builds at release. A `#[cfg(target_os = "macos")]` block can still slip through if a code path isn't exercised. **Assume cross-platform CI is the source of truth, and validate installers locally (see "Installer build pitfalls") before tagging.**

**First tell a transient infra failure apart from a real one.** If a job failed only because a download 504'd (the cargo-dist installer asset, a `curl`/registry blip) — not a compile/test error — wait for the asset to return 200, then `gh run rerun <run-id> --failed`; do NOT bump the version. A re-run that goes green flips the run to success and re-fires `crates-publish`. Use the patch-bump steps below only for genuine code/build failures.

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
6. **Re-run the ship steps** (6–9 of the standard workflow): verify locally, commit, merge to `main` (→ crate), then push the **new** tag `vX.Y.Z` (→ release + installers). Verify the crate + all 32 assets after.
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
     - Medium-risk actions: require confirmation unless explicitly auto-confirmed with --yes
     - High-risk actions: render structured RiskExplanation prompt; require explicit 'y'
     - Apply, capture ActionOutcome, sleep stabilization window
     - If fatal_environment_change → break out of plan, re-probe immediately
6. Re-run diagnostics. If now passing → done. Else continue to next iteration.
7. Bounded by: ≤6 iterations, ≤4-min wall clock, per-action max_attempts
```

### Module layout under `src/actions/fix/`

- **`action.rs`** — `Action`, `ActionId`, `Cost`, `Risk`, `Reversibility`, `RiskExplanation` types. The registry (`all_actions()`) returns every `Action` available on the current platform. `Action::apply(&self, config, restore: &RestoreRegistry)` dispatches to a free function per `ActionId` via match; destructive `apply_*` fns register their inverse op on the registry before mutating state. `Risk::High(RiskExplanation)` is a sealed enum variant: a high-risk Action *cannot* be constructed without a complete plain-language explanation.
- **`triage.rs`** — Pure planning. `actionable_failures()`, `group_by_root_cause()` (walks the hardcoded dependency DAG), `hard_block_detected()`, `build_plan()`. No IO. Unit-tested via `cargo test --lib actions::fix::triage`.
- **`session.rs`** — `Session` (per-run state: attempts, effectiveness, snapshots, action_log) + `Reporter` (plain-language output for stdout) + `FinalOutcome` (incl. `Interrupted` → exit 130) + the **`RestoreRegistry`** / `RestoreOp` / `restore_op` interrupt-safe restore machinery (see "Interrupt-safe restore"). `WALL_CLOCK_CAP = 240s`.
- **`loop_runner.rs`** — `run_and_finalize(config) -> i32`: pre-flight elevation check, constructs `Session` + `RestoreRegistry`, races the caught-unwind loop against Ctrl-C, SIGTERM, and SIGHUP on Unix, then **always drains LIFO** under a dynamic outer budget of 30 seconds per pending inverse plus 5 seconds before persisting the report. Interrupt → exit 130; panic → drain then exit 101. JSON includes `interrupted` + `manual_recovery_needed`.
- **`report.rs`** — Iteration-oriented Markdown report. Under `sudo`, it targets the validated invoking user's `~/Downloads`, refuses unsafe directory state, and atomically creates a mode-`0600` file with the correct ownership. Drain failures add "Manual recovery needed".
- **`stages.rs`** — Platform repair primitives. macOS `DeepStackReset` is now a fail-closed, high-risk network-service **cycle**, never deletion/recreation: map exactly one enabled physical service, snapshot DNS servers/search domains, register `ReEnableMacosService` before disabling, cycle off/on, verify enabled state plus the expected physical route/reachability, then resolve the token. The production gate remains false until the ignored live acceptance test passes under root plus exact acknowledgements and an independent offline watchdog on a disposable VM/service. Do not test it on a Mac whose Codex/SSH control depends on the selected link. The old service snapshot/password/SSID/deletion code is forbidden by regression tests. DHCP renewal must first prove the selected service already uses DHCP; Manual/BootP/static/unknown modes are no-op.
- **`cmd.rs`** — bounded read-only command helpers plus an owned mutation runner that serializes state changes and kills/reaps a timed-out or cancelled child before restoration. Paired down/up, off/on, release/renew, and stop/start supervisors always attempt the inverse and prevent a late first child from racing recovery. `run_cmd_capture` returns structured `CmdOutcome` payloads for reports.

### High-risk prompts

In interactive mode, every High-risk Action renders a multi-line block with `what`, `why`, side-effects, reversibility, and typical duration before running. Requires an **explicit `y`** — Enter alone, blank, or any non-`y` is treated as N. **`--yes` does NOT bypass these prompts** (by design — protects non-technicians from accidentally OK'ing destructive actions). In `--json` / non-interactive contexts, High-risk Actions are *skipped* (not auto-applied) with a clear marker in the report.

### Interrupt-safe restore (v3.0.9+)

`nd300 fix` is safe to cancel / time out / crash mid-repair. A non-poisoning **`RestoreRegistry`** threads through `Action::apply`; every inverse is registered *before* mutation. `RestoreOp` includes `ReEnableInterface`, `ReEnableVpn`, and macOS-only `ReEnableMacosService`; there is no service-recreation restore. Pending operations drain newest-first after Ctrl-C/SIGTERM/SIGHUP/panic/normal termination, with a 30-second per-op timeout and an outer budget sized to the pending LIFO stack. Consumer VPN inverses are registered before disconnect and clear only after positive provider/OS state readback; enterprise VPNs are never touched. All supervised mutation children are serialized, terminated, and reaped before an inverse runs, so a late disconnect/down/stop/release cannot race recovery. A token is resolved, and `reverted: true` emitted, only after state verification.

### What to do when adding a new fix primitive

1. Add an `ActionId` variant in `action.rs`.
2. Add a free `apply_*` function for it (take `&RestoreRegistry` if it mutates restorable state).
3. Add a match arm in `Action::apply`.
4. Append an `Action { ... }` entry in `all_actions()` declaring `targets`, `cost`, `risk`, `max_attempts`, `stabilization`. For `Risk::High`, attach a complete `RiskExplanation`.
5. Triage planning picks it up automatically — no changes needed in `triage.rs`.
6. If the primitive makes a destructive, restorable change: add a `RestoreOp` variant (cfg-gate platform-only types), exhaustive-per-platform `restore_op`/`label` arms, then `register` before mutating and `mark_resolved` after restoring.

Enterprise VPNs (Cisco, Zscaler, Palo Alto / GlobalProtect, F5, Check Point, Juniper) are never auto-disabled. Fix reports go to the validated invoking user's `~/Downloads/nd300-fix-report-YYYYMMDD-HHMMSS.md`, never `/var/root` merely because `sudo` was used.

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
4. Prove exactly one channel: Cargo registry metadata; a validated Unix archive receipt; the exact macOS PKG receipt/payload/root-owned hash-bound pair; Windows Cargo/standalone; or one registered MSI/EXE edition. Package-manager/local-build/unknown/conflicted ownership is refused.
5. Keep public entrypoints versionless, but pin the resolved tag for every internal artifact URL, checksum, and post-install comparison in this transaction.
6. Execute only the matching channel strategy. Cargo runs bounded as the validated invoking user and never runs `rustup update`; Unix managed archives use the exact target archive; a macOS PKG downloads its exact-tag versionless direct PKG and invokes Apple Installer; Windows uses the exact standalone/MSI/EXE route.
7. Verify both programs at the exact expected version. Unix archives additionally use bounded/size-limited download, in-Rust SHA-256, strict two-file extraction, and macOS pinned Developer ID/team/identifier/runtime/timestamp verification. The package route verifies the direct PKG identity, trusted timestamp, notarization ticket, Gatekeeper install policy, exact payload/PackageInfo, installed receipt, hashes, and pair after Installer closes.
8. Transactionally swap/retire both binaries and restore both on every partial failure. Windows MSI cleanup is a commit action, Inno restores in `DeinitializeSetup`, and generated PowerShell restores around its copy loop.
9. Never cross channels after a failed update. Report the attempt and the manual install URL while preserving the old owner.
```

### Key Constants (to customize per tool)

```rust
const RELEASES_URL: &str = "https://api.github.com/repos/{OWNER}/{REPO}/releases/latest";
const RELEASE_BASE: &str = "https://github.com/{OWNER}/{REPO}/releases/download";
const PS_INSTALLER_ASSET: &str = "nd300-installer.ps1";
// exact URL: format!("{RELEASE_BASE}/v{resolved}/{PS_INSTALLER_ASSET}")
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

- **Windows**: The running `.exe` cannot be overwritten, but Windows permits an in-directory rename. Writable installs retire both binaries to versioned allowlisted siblings before strategy execution and restore them if every strategy fails. MSI performs the same move with the rollback-aware `MoveFiles` action before `RemoveExistingProducts`; Inno uses `PrepareToInstall` plus `DeinitializeSetup` restoration and disables Restart Manager shutdown; release automation fail-closed patches cargo-dist's generated PowerShell installer around its binary copy loop. The new binary runs hidden `migrate-cleanup --retired-update`, which removes non-running siblings immediately and schedules the still-running image for trusted delayed deletion. Keep all four paths and the generated-script anchors in lockstep.
- **macOS/Linux**: `unix_install.rs` owns secure temporary directories, exact-tag archive/package download and verification, safe extraction, paired archive replacement/rollback markers, invoking-user Cargo execution, and origin classification. The archive route retains pinned Developer ID binary checks. The macOS package route requires `com.qubetx.nd300.pkg`, exact payload/PackageInfo, root-owned non-writable `/usr/local/bin` pair and hash metadata, Developer ID Installer identity/timestamp, a stapled ticket, and Gatekeeper install acceptance. It launches the direct PKG with `open -W -a Installer`; cancellation or mismatch is failure, never a cross-channel fallback. The release-only compatibility DMG is not part of new updater execution.
- **Unix uninstall**: Cargo origin → `cargo uninstall nd300`; ordinary invocation symlink → remove only the symlink; valid cargo-dist receipt → allow the matching managed files; proven macOS PKG → authorize removal of only the pair/metadata then forget the receipt; package-manager/local-build/unknown → refuse. Never guess from location alone.
- **Cargo**: bare `cargo install` has no project post-install hook, so it cannot uninstall an unrelated registered product. A Cargo-managed update remains Cargo-only and pins `--version =<resolved> --locked`; users deliberately switching from a Windows installer to bare Cargo must uninstall the registered owner first. Official MSI/EXE/PowerShell fresh installers can perform same-scope takeover and must refuse an opposite-scope owner before mutation.

### JSON Output

Supports `--json` mode with structured output matching the uninstall pattern.
Stdout contains exactly one final JSON object; transaction progress and child
status use stderr. Existing fields remain compatible. Every result adds
`install_channel`, `recovery_url`, and `requires_user_action`; a failed known
installer strategy adds `exact_installer_url`. Windows also retains top-level
`install_origin` (one of `msi-global`/`msi-corporate`/`exe-global`/
`exe-corporate`/`cargo-or-installer`/`unknown`):
```json
{
  "action": "update",
  "success": true,
  "current_version": "3.5.2",
  "latest_version": "3.7.3",
  "update_available": true,
  "method": "installer",
  "strategy": "msi_corporate",
  "install_origin": "msi-corporate",
  "install_channel": "msi-corporate",
  "recovery_url": "https://github.com/QubeTX/qube-network-diagnostics/releases/latest",
  "requires_user_action": false
}
```

## macOS Universal Direct PKG (v3.7.3+)

`macos-installer.yml` is both the internal-PR candidate gate and the exact-tag
reusable release job. It builds native thin pairs on `macos-15` and
`macos-15-intel`, creates one universal pair, re-signs each Mach-O with its
pinned Developer ID Application identity/hardened runtime/timestamp, and
packages both plus hash-bound install metadata as the preferred versionless
`nd300-universal-apple-darwin.pkg` with Developer ID Installer. The PKG is
signed, notarized, stapled, Gatekeeper-assessed, checksummed, and attested
directly. A separately signed/notarized/stapled compatibility DMG contains
those exact PKG bytes for immutable v3.7.1/v3.7.2 updaters.

Fresh PKG installation is authoritative and may repair the same version or
downgrade deliberately; automatic update is latest-only and same-channel. The
postinstall hook runs the new package binary as the console user with root/Cargo
environment removed and consolidates only a registry/receipt-proven standard
Cargo-home pair. Ambiguous or nonstandard copies are retained. Hosted candidate
and release validation cover both architectures, managed-archive takeover,
same-version repair, a private higher-receipt downgrade fixture, both output
modes, update selection, uninstall/receipt removal, and reinstall. The private
fixture is never published. Release publication refuses overwrite, adds the
direct PKG, compatibility DMG, and both sidecars, attests all four to the exact
tag SHA, and requires the canonical 32-asset inventory before announcement.

## Windows Installer Matrix + Installer-Aware Self-Update (v3.1.0+)

ND-300 ships **four first-class Windows installers**, each packaging **both** `nd300.exe` and `speedqx.exe`:

| Edition | Format | Source file | Scope | PATH | Install dir | Marker value |
|---|---|---|---|---|---|---|
| Global | MSI | `wix/main.wxs` (cargo-dist) | perMachine (admin) | system (HKLM) | `C:\Program Files\nd300\bin\` | `msi-global` |
| Corporate | MSI | `wix-corporate/corporate.wxs` | perUser (no UAC) | user (HKCU) | `%LocalAppData%\Programs\nd300\bin\` | `msi-corporate` |
| Global | EXE | `inno/global.iss` (Inno Setup) | perMachine (admin) | system (HKLM) | `C:\Program Files\nd300\bin\` | `exe-global` |
| Corporate | EXE | `inno/corporate.iss` (Inno Setup) | perUser (no UAC) | user (HKCU) | `%LocalAppData%\Programs\nd300\bin\` | `exe-corporate` |

The Global MSI is the cargo-dist base asset. The other **6 assets** (Corporate MSI + 2 EXEs + 3 `.sha256` sidecars) are built by `.github/workflows/windows-installers.yml`. **Two binaries:** every installer must keep BOTH `binary0`=nd300.exe and `binary1`=speedqx.exe (Inno: two `[Files]` lines). Both Inno scripts show the committed root `LICENSE` in the wizard.

### Marker → update dispatch + the lockstep contract

Each installer writes `HKCU\Software\ND300\InstallSource = <marker value>`, but the marker is a hint rather than unconditional authority. Inno owns it through its Registry entry; MSI writes it with `install-maintenance` only as a commit action and clears it on uninstall only if it still names that channel, avoiding a shared component GUID across products. `registered_install_owner()` scans both HKCU/HKLM uninstall views, requires the expected display/scope/format, normalizes `InstallLocation` against the running binary, and reduces the command to either an MSI product GUID independently proven by `MsiEnumRelatedProductsW` under a canonical UpgradeCode or a known Inno uninstaller inside that location. A `.cargo\bin` path always wins; a unique proven owner wins over a contradictory marker; the marker is accepted only when its edition matches the path fallback. `build_strategy_list()` returns a **single** matching MSI/EXE strategy. The resolved release pins every artifact and sidecar URL; downloads remain SHA-256 verified and post-install version checked.

**LOCKSTEP CONTRACT (do not break):** install paths, marker values, display names, MSI scope/UpgradeCodes, the two fixed Inno AppIds, versioned retired-image names, and hidden cleanup/maintenance arguments in all four installer files must stay aligned with `update.rs`, `takeover.rs`, `migrate.rs`, `maintenance.rs`, and the cargo-dist PowerShell post-processor. All four must package both binaries. Run the installer lockstep reviewer after any change.

**Update and fresh-install intent are different contracts.** `nd300 update` preserves its proven channel. A fresh official Windows installer invokes hidden `install-takeover --target <channel> --scope all` and may replace only a proven owner in the same per-user/per-machine scope. Same-scope MSI↔Inno changes invoke the Windows-Installer-proven product code or validated Inno uninstaller directly, without a shell. An opposite-scope owner is refused before mutation: Windows Installer cannot major-upgrade across contexts, and elevated machine setup must never execute an HKCU/user-writable uninstaller. The selected install claims ownership only after supported conflicts are gone. Default `migrate-cleanup` still handles Cargo-copy and advisory file cleanup without touching Cargo/Rustup/PATH/ARP/receipts; `--retired-update` is a commit action in MSI and post-success action in Inno. The Global MSI commit sequence also repairs the exact pre-v3.1 system-PATH entry while preserving unrelated values and registry type.

Top-level Windows uninstall uses the proven registered owner so both binaries, ARP registration, correct PATH hive, and marker leave together; raw registry command strings never go through a shell. Cargo/portable fallback retains the existing two-binary allowlist. JSON fields and strategy IDs remain compatible.

### `windows-installers.yml` — tag-triggered (consistent with TR-300)

ND-300's `Release` workflow fires on a `vX.Y.Z` **tag push**, so a tag-triggered Release run has `head_branch == the tag name`. `windows-installers.yml`:
- Is a reusable `workflow_call` invoked by the tag-push Release job with its validated `tag` and `source_sha`; it no longer uses `workflow_run` (whose SHA is the default branch). Manual `workflow_dispatch` remains repair-only and must itself run at `--ref <tag>`.
- **Resolves the immutable tag**, requires it to match the caller/manual SHA, and checks out that exact commit. The Release announcement requires this reusable job to succeed.
- **Pre-flight + idempotency:** probes the release for `dist-manifest.json` + `nd300-x86_64-pc-windows-msvc.msi` (torn-release guard); if all 6 corporate/EXE assets are already attached on a non-dispatch run, logs and exits 0. `workflow_dispatch` always rebuilds (`--clobber`).
- Keeps the WiX `candle`/`light -sice:ICE38 -sice:ICE64 -sice:ICE91` + Inno `iscc /DMyAppVersion=` mechanics. Windows verification completes the cargo-dist base assets + these 6 at 28; the later reusable macOS job adds four for the final 32.

`Cargo.toml`: `allow-dirty = ["ci", "msi"]` (the `msi` is for the customized `wix/main.wxs` update, rollback, takeover, marker, and PATH maintenance), `/wix-corporate/**` + `/inno/*.iss` in the `include` list (never generated `inno/Output`), and the Windows-only `sha2`/`winreg` deps.

### Installer build pitfalls + CI gate

cargo-dist hides WiX `candle`'s real error (just "exit 104"), so validate all four installers **locally** before shipping. `.github/workflows/ci.yml` runs fmt/clippy/test/build on macOS/Linux/Windows plus permanent aarch64 GNU and x86_64 musl release-target compile gates. `.github/workflows/windows-origin-matrix.yml` builds normal + fault installers once, exercises the five current origins, the pre-marker v2.9 Global MSI path, four same-scope fresh format changes, Cargo→Global MSI, and an opposite-scope refusal whose before/after fingerprint must remain identical; dispatch it in `public` mode after release. `release.yml` remains the source of truth for signing/notarization, all six artifacts, exact-SHA attestations, and publication.

**cargo-dist is installed from the prebuilt binary tarball, not `installer.sh`.** `ci.yml`'s `dist-plan` and `release.yml`'s `plan` job fetch `cargo-dist-x86_64-unknown-linux-gnu.tar.xz` (`curl --retry`) and extract `dist` directly (it needs `chmod +x`), because GitHub's release CDN has repeatedly 504'd the small `cargo-dist-installer.sh` asset while the `.tar.xz` stayed up — and `dist-plan` is not `continue-on-error`, so that flake blocks the crates.io publish (it held up v3.3.1 for hours; fixed in v3.3.2). Don't revert these to `curl installer.sh | sh` (a `dist generate` would; re-apply the tarball install if so). Per-target `build-local-artifacts` jobs still use cargo-dist's platform-detecting installer (each runner needs a different host tarball).

### `nd300 dns` subcommand

`Nd300Command::Dns` (`src/cli.rs`) is the bare-subcommand form of `-d`/`--dns`. In `src/main.rs` it is routed through the **same semi-exit-early path** as the flag (sets a `run_dns` intent rather than the terminal subcommand block), so `nd300 dns` ≡ `nd300 --dns` (exit on failure, fall through to diagnostics on success). All other subcommands stay terminal.

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

---

## Subagent model preference (applies to ALL subagents in this repo)

When working in this repository, whenever you spawn a subagent — the `Agent` tool (**including Explore
and Plan agents**), `Workflow` agents, or any other subagent — use a top-tier model/effort pairing.
**Never** leave a subagent on a lower model or below `xhigh` effort.

**Per model — pick the model that fits the task, then its effort. Prefer the first; never go below either.**
- **Opus 4.8 [1m]:** `xhigh` **preferred** (most situations); `max` allowed when the agent judges it
  needs the deeper reasoning (e.g. the single hardest stage).
- **Sonnet 5:** `max` **preferred** (most situations); `xhigh` allowed when the agent doesn't need the
  extra thinking (e.g. cheaper/faster mechanical fan-out).

Only the **Opus** and **Sonnet** classes are in scope. **Never** a weaker/budget class (no **Haiku** or
older), and do **not** substitute the **Fable ("mythos")** class or any other/new class into the Opus or
Sonnet slot just because it's new or capable — adopting a different class is a deliberate change to this
convention, not an automatic remap. **Never below `xhigh`** effort. Which model + which of its two efforts
is the spawning/orchestrating agent's call per situation — just honor each model's preferred default and
the floor.

In `Workflow` scripts pass model + effort explicitly per agent (`{model:'opus', effort:'xhigh'|'max'}` or
`{model:'sonnet', effort:'max'|'xhigh'}`); for the `Agent` tool set `model` to `opus`/`sonnet` (it inherits
the session's `[1m]` context/effort).

---

## Mapping this forward (when new models are released)

This convention names **two specific model classes** — **Opus** and **Sonnet** — plus each one's **role**
and an **effort floor**. It is not tied to version numbers. When Anthropic ships a new lineup, advance
**each named class along its own lineage** (Opus → next Opus, Sonnet → next Sonnet) and keep it in role:

- **Opus class — flagship / deepest reasoning.** Today **Opus 4.8 [1m]** → the newest **Opus-class**
  model in its **largest-context** variant. Keep **`xhigh` preferred, `max` when needed**. Role: deep
  synthesis, planning, verification, the single hardest stage.
- **Sonnet class — workhorse / high-throughput.** Today **Sonnet 5** → the newest **Sonnet-class**
  model. Keep **`max` preferred, `xhigh` when lighter**. Role: high-parallelism fan-out, mechanical/bulk work.
- **Only the Opus and Sonnet classes are in scope — mind the other classes.** **Haiku** is the excluded
  budget class (never use it for subagents). The **Fable ("mythos") class** — and any other or brand-new
  class — is **not** one of these two slots: do **not** silently map a Fable/mythos model into the Opus
  or Sonnet role just because it's new, large, or capable. Adopting a different class is a *deliberate*
  update to this convention, never an automatic role-remap.
- **The floor holds regardless of names:** never a class below Sonnet (no Haiku/older), and **never
  below `xhigh`** effort.

**At each release, do this:**
1. Find the current **Opus-class** and **Sonnet-class** models (same class lineage as today). Ignore
   Haiku, and ignore any other class (e.g. Fable/mythos) unless this convention is explicitly updated to include it.
2. Swap in the new Opus-class and Sonnet-class names; keep each class's preferred/allowed efforts and the floor.
3. If the effort-level names change, preserve the *shape* on the effort ladder: Opus defaults to
   **one below the top** and may go **top**; Sonnet defaults to the **top** and may drop **one below**.
   The floor stays at "one below the top" (today = `xhigh`) — never lower.
4. Confirm the exact model IDs and the long-context suffix (today `[1m]`), and update the
   `{model:'opus'|'sonnet'}` tool aliases if the class keywords change. (Check current model docs /
   the `claude-api` reference.)
5. Bump the "Set …" date.

**Rule of thumb:** _Opus + Sonnet classes only (never Haiku, never auto-adopt Fable/mythos), top-ish
effort, never below the second-highest effort._
