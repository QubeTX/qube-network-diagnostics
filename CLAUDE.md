# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

ND-300 (`nd300`) is a cross-platform CLI network diagnostics tool written in Rust. It runs 25+ diagnostic modules concurrently, supports user and technician modes, and includes a diagnostic-driven network recovery system (`--fix`). Ships two binaries: `nd300` (main diagnostic) and `speedqx` (standalone speed test).

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

### Skill routing and current guidance

Use `/tasks-start` to initialize, repair, upgrade, relaunch, or resume the board.
`/tasks-create` is the preferred way to add a well-formed milestone, task, or proper
dashboard-visible subtask; `tasks-management` is the format and completion contract.
Use `/tasks-update` to upgrade and reconcile the existing board, sync/triage current
work, and refresh memory. `tasks-memory` governs that memory, `tasks-boards` governs
live-server identity, and `/tasks-remove` decommissions the system. As work changes,
keep `.tasks/TASKS.md` plus each Active task's `## Status` and `## Activity` current.

If the installed tasks plugin is missing or may be older than the board, first try the
harness-native plugin update. If that is unavailable, fails, or still leaves freshness
uncertain, use the GitHub skill/connector to read the relevant current `main` file under
`RealEmmettS/shaughv-tasks/skills/<skill-name>/SKILL.md` and use it as the latest
operating guidance: https://github.com/RealEmmettS/shaughv-tasks/tree/main/skills

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
cargo run --bin speedqx -- --duration 5 --fastcom-duration 5 --no-color   # quick live smoke of all 8 providers
cargo run --bin nd300 -- -t --fast --no-color   # live smoke of all 25 deep modules without the saturating phases
```

Security advisories: `cargo audit` (install once via `cargo install cargo-audit --locked`) auto-reads the project's `.cargo/audit.toml` ignore-list — when an advisory can't be cleared by a dep bump, add it there with a rationale instead of re-triaging. On a Windows host, use `cargo tree --target all -i <crate>` to see platform-gated transitive deps (e.g. the Linux netlink stack that pulls `paste`); plain `cargo tree` only shows host-target deps.

Tests cover the fix planner and confirmation rules, platform command construction, latency argument selection, and speed-test aggregation. CI release builds remain the source of truth for all 6 platform targets.

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
    → tech mode (-t): T1 = 21 deep (concurrent, boxed futures) → T2 = route_path ∥ packet_loss ∥ path_mtu (long, latency-sensitive) → T2b = nat + arp_health (derived, free) → T3 = bufferbloat (sequential, saturating) = 25 total, uses SharedCache
  → render/ (user_mode | tech_mode | json — selected by format + mode)
  → Exit code: 0=OK, 1=Warn, 2=Fail
```

Diagnostic counts: README/CHANGELOG cite the canonical totals (**8 core**, **25 deep**). The flow box splits each total into its concurrent groups plus the sequential outliers (speed test for core; bufferbloat for tech) — same modules, just grouped by how they run. Keep these reconciled when adding a module. Tech-mode module futures are **boxed** inside the `tokio::join!` — a flat 21-way join of inlined async state machines overflowed the Windows main-thread stack.

`Nd300Cli` defines a `command: Option<Nd300Command>` field alongside the legacy boolean flags. If a subcommand is given, it takes precedence; otherwise the flag-form dispatch runs. Both forms produce identical behavior. Output flags (`--json`, `--ascii`, `--no-color`, `--verbose`), fix confirmation (`--yes`), and speed controls (`--fast`, `--speed-duration`) are marked `global = true` so they work in both positions. `Nd300Command::Dns` is special: it is routed through the same *semi*-exit-early path as the `-d`/`--dns` flag (exit on failure, fall through to diagnostics on success) rather than the terminal subcommand block, so `nd300 dns` ≡ `nd300 --dns`. All other subcommands are terminal.

### Module Layout

- **`src/cli.rs`** — Shared clap derive definitions (`Nd300Cli`, `SpeedQXCli`) used by both binaries and by `build.rs` for man page generation.
- **`src/actions/`** — Exit-early operations (`fix`, `clear-dns`, `uninstall`, `dns`, `update` — each available as a bare subcommand and a legacy flag; plus hidden installer helpers). `dns` is semi-exit-early. The fix flow implements evidence-driven recovery, LIFO verified restoration, exact macOS DNS/search rollback, and invoking-user-owned private reports. The updater (`update.rs` + Unix-only `unix_install.rs`) proves one owner and executes only that channel: Cargo, managed Unix archive, receipt/hash-proven direct macOS PKG, Windows standalone, or exact MSI/EXE edition. `takeover.rs` performs validated same-scope fresh Windows replacement; `migrate.rs` owns installer-authorized Cargo/edition/retired-file cleanup; `maintenance.rs` owns MSI commit-time markers and pre-v3.1 Global PATH repair. Registered Windows uninstall delegates to a proven MSI/Inno owner; macOS package uninstall removes only its proven pair/metadata and receipt; unknown/package-manager/local-build Unix locations are refused.
- **`src/actions/migrate.rs`** — Cross-method install cleanup, exposed as hidden `nd300 migrate-cleanup`. Windows origins keep the existing synchronous/locked pair cleanup and exact retired-image allowlist. The Apple postinstall passes hidden `--install-origin macos-pkg --cargo-copy` as the console user with root/Cargo environment removed; only an exact Cargo registry or cargo-dist receipt in that user's standard Cargo home may remove the older pair. Ambiguous files remain. Cleanup stays advisory, never escalates, never deletes Cargo/Rustup/unknown files/the running package pair, and exits 0 except for a true internal error.
- **`src/diagnostics/`** — All diagnostic modules. Core/deep contracts and pure fixture parsers remain. On macOS, `interfaces.rs` builds one topology truth from default route + current flags + hardware-port/service mapping + addresses; `wifi.rs` uses bounded `system_profiler` only in technician mode; VPN requires correlated configuration/state/address/route evidence; IPv6 separates ULA and verifies both stacks; TLS/DNSSEC are evidence-qualified. Modern/legacy fixtures cover route/listener/connection/DHCP/protocol/Wi-Fi/service/VPN formats. `util.rs` timeout wrappers kill and reap expired child processes.
- **`src/speedtest/`** — Methodology v5 acquisition (`acquisition_v5.rs`), pure counter interpretation (`measurement_v5.rs`) and bounded common orchestration (`engine_v5.rs`). ND300 and speedqx default to Quick; Deep repeats Cloudflare/MSAK primaries and adds supplementary providers. M-Lab requires explicit consent; NDT7 is separately labeled single stream. The retained v4 providers/statistics are reference and historical code. Full recorded-trace parity uses `examples/v5-replay.rs` and the pinned TypeScript contract; no unsupported confidence-driven early stop affects v5.
- **`src/render/`** — Output formatting. `table.rs` builds Unicode/ASCII box-drawing tables with ANSI-aware string functions (`visible_len`, `truncate_visible`). `color.rs` centralizes ANSI color output. `progress.rs` handles spinners.
- **`src/config.rs`** — Config builder with fluent API. The Config object threads through the entire render pipeline.
- **`src/error.rs`** — Unified `AppError` enum via `thiserror`.
- **`src/platform/`** — OS detection/elevation plus Unix `invoking_user.rs`, validating numeric `SUDO_UID`/passwd data and supplying original-user home, Cargo paths, ownership, and privilege-dropped commands.

### Key Patterns

**Platform abstraction:** Compile-time `#[cfg(target_os)]` branching per module — no runtime OS checks. Windows uses `winapi` (GetIfEntry2, TCP/UDP tables), `ipconfig` crate, and WMI (tech-mode only). Unix uses `nix` and `libc`.

**Diagnostic module contract:** Core modules export `pub async fn check() -> (DiagnosticResult, Option<DetailType>)`; deep modules export `pub async fn collect() -> Option<DetailType>` (or `collect_with_cache(&SharedCache)`), where `None` = Skip (section omitted) and verdicts live inside the detail struct (`assessment` + `level: "ok"|"warn"|"fail"` fields by convention). Core verdict logic is extracted into pure `*_verdict()` functions so thresholds are unit-testable without a network.

**Verdict stability (v3.4.0+):** core checks require *consistent* failure before reporting Fail — gateway runs a 3-ping burst plus a conditional second burst (Fail = 0/6); DNS probes 3 independent domains with one retry each and judges on count-resolved + median time; ports test 2 endpoints per port with 2 attempts each (open if either connects, unresolved excluded from the denominator); latency's average-based Warns require ≥2 reachable targets. Timeouts still map to failed attempts (never remapped to Warn — that would mask outages); de-flaking comes from the consistency requirements. Probe results are deliberately NOT shared across core modules (shared samples would correlate verdicts).

**Subprocess timeouts:** Fix commands retain explicit timeout classes. Diagnostic subprocess/DNS calls are bounded by `diagnostics/util.rs`; a timed-out child is killed and reaped, then callers receive the existing `None` failure shape. Apple networkQuality adds progress heartbeats, a whole-phase deadline, and capped exponential backoff with jitter. `run_all(config, cap)` still returns partial results and fabricated timed-out rows at its internal cap, and triage skips those fabricated failures.

**SharedCache (tech mode):** `diagnostics/shared_cache.rs` pre-fetches netstat, ipconfig, sysinfo::Networks, and default gateway data once, shared across 10+ tech-mode diagnostic modules. Built via `SharedCache::build_for_tech_mode()` before the tech diagnostic `tokio::join!`. `run_technician_diagnostics(config, public_ip)` also receives the core public-IP details so the NAT analysis doesn't re-fetch.

**DiagnosticStatus enum:** `Ok`, `Warn`, `Fail`, `Skip` — only the 8 core diagnostics (the 7 concurrent + speed test) contribute to exit code aggregation; tech-mode deep diagnostics do not.

**Build script:** `build.rs` generates man pages (`man/nd300.1`, `man/speedqx.1`) at build time via `clap_mangen`. It uses a `#[path = "src/cli.rs"]` include with a stub `speedtest` module to compile the CLI structs in the build context.

### Dual-Binary Design

Both `nd300` and `speedqx` use the shared `speedtest` module. CLI definitions live in `src/cli.rs` (not in the binary files). The speedqx binary (`src/bin/speedqx.rs`) has its own progress display via `DisplayState` + `SpeedQXDisplay`, tracking phases (CfLatency → CfDownload → CfUpload → Ndt7Discovery → Ndt7Download → Ndt7Upload → Computing).

### Cross-Platform Build Targets

aarch64-apple-darwin, x86_64-apple-darwin, aarch64-unknown-linux-gnu, x86_64-unknown-linux-gnu, x86_64-unknown-linux-musl, x86_64-pc-windows-msvc

## Repo-local agent tooling

Project automation is shared across supported coding agents. `.claude/` is the
Claude Code configuration, `.agents/skills/` exposes the portable release and
installer-validation skills, and `.codex/` contains Codex reviewer definitions,
portable hooks, and the Context7 MCP declaration. The equivalent skill,
reviewer, and hook bodies must remain behaviorally aligned when one copy changes;
manifests may differ only where the host products require different schemas or
paths. Never commit user-specific absolute paths or secret values. Generated
installer binaries under `inno/Output/` are ignored build output, not source.

## Release Process

Releases use **cargo-dist v0.31.0, tag-triggered**, plus a CI-gated crates.io publish — the same build+deploy cycle as TR-300. Five workflows:
- **`ci.yml`** — quality gate: fmt + Rust 1.97 clippy/test/release build on macOS arm64 + Intel, Linux, and Windows (`-D warnings`), plus blocking audit, `dist plan`, and read-only Mac smoke. Runs on every PR and `main` push.
- **`crates-publish.yml`** — publishes the `nd300` crate, fired via `workflow_run` after `CI` succeeds on a `main` push. Idempotent (skips a version already on crates.io). This is the ONLY place the crate is published.
- **`release.yml`** — cargo-dist tag release. arm64 Mac runs on `macos-15`, Intel on `macos-15-intel`, preserving deployment floors 11.0/10.12. Before hosting, an ephemeral-keychain script signs both binaries in both archives by the sole pinned fingerprint, verifies Team ID/identifier/hardened runtime/timestamp, notarizes both binaries together, requires Apple status `Accepted`, repacks exact signed bytes, and regenerates sidecars/manifest. Missing credentials or any mismatch fails closed; no unsigned fallback. Host gets write/id-token only here and attests final post-signing assets.
- **`windows-installers.yml`** — builds Corporate MSI + two Inno EXEs + sidecars, separately attests them, and verifies the 28-asset Windows-complete inventory.
- **`macos-installer.yml`** — internal PRs build/sign/notarize and run the direct universal PKG lifecycle on native Intel and Apple Silicon, while proving the legacy DMG contains byte-identical PKG bytes. The tag-bound reusable job starts from the exact signed architecture archives, adds the versionless PKG + sidecar and compatibility DMG + sidecar, attests all four to the tag SHA, refuses overwrite, verifies the final **32 assets**, then installs immutable v3.7.2 and exercises the compatibility package transition on both native architectures before announcement. Workflow defaults stay `contents: read`.

**So a deploy is two pushes: merge to `main` (→ crate) then push a `vX.Y.Z` tag (→ binaries/installers/release).** The WiX manifest (`wix/main.wxs`) must include components for both `nd300.exe` and `speedqx.exe`; every installer ships both binaries.

Mac release credentials are repository secrets `APPLE_CERTIFICATE_P12_BASE64`, `APPLE_CERTIFICATE_PASSWORD`, `APPLE_INSTALLER_CERTIFICATE_P12_BASE64`, `APPLE_INSTALLER_CERTIFICATE_PASSWORD`, `APPLE_API_KEY_P8_BASE64`, `APPLE_API_KEY_ID`, and `APPLE_API_ISSUER_ID`, plus variables `APPLE_SIGNING_IDENTITY=739B04530883FF9B665C66BD464F98C622971B32`, `APPLE_INSTALLER_SIGNING_IDENTITY=Developer ID Installer: ES Development LLC (M9D5379H93)`, and `APPLE_TEAM_ID=M9D5379H93`. Never print/commit secrets. Both native candidates must prove the G2 RSA-2048 Installer certificate and private key by signing a disposable PKG.

### Standard release workflow (run when shipping) — tag-push, consistent with TR-300

Triggers in user prompts: "release", "ship it", "deploy", "tag the release", "push it out", "make this live", "cut a release", "go for it" (after change discussion).

1. **Bump the version in `Cargo.toml`** (`[package] version`) — patch for fixes, minor for backward-compatible features, major for breaking changes. Always a never-published version (forward only). This ONE number drives both the crate (crates-publish.yml on the `main` merge) AND the GitHub/installer release (the `vX.Y.Z` tag) — bump it once.
2. **Update `CHANGELOG.md` + `HUMAN_CHANGELOG.md`** in lockstep (every technical entry needs a plain-English counterpart — see "Changelog rule"). Date = the host's current date.
3. **Update `README.md`** (user-visible flags/commands/behavior) and **`CLAUDE.md` / `AGENTS.md`** (architecture/workflow changes).
4. **Bump the homepage version fallbacks** in the `qube-machine-report-homepage` repo — the `useGitHubVersion('QubeTX/qube-network-diagnostics', '<new>')` calls in `ND300Install.jsx`, `ND300Hero.jsx`, `ND300Footer.jsx`, and `InstallGuideContent.jsx` (the live GitHub fetch overrides on a healthy network; this keeps the offline fallback current). Ship as its own PR → merge to `main` → Vercel auto-deploys.
5. **Local verification:** `cargo fmt --all -- --check`, Rust 1.97 `cargo clippy --locked --workspace --all-targets -- -D warnings`, `cargo test --locked --workspace --all-targets`, `cargo build --release --locked`, `cargo package --locked --list`, `cargo publish --dry-run --locked`, `cargo audit`, `dist plan`, actionlint, ShellCheck, archive/man inspection, updater fault tests, Windows installer validation, and read-only smokes. Hosted native Intel/ARM direct-package candidate lifecycle, accepted PKG/DMG notarization, byte identity, exact-SHA release, all 32 assets/attestations, public updater matrices, and the physical Alienware Windows update are release gates until evidenced. A physical-Mac visual repeat is supplementary; the required Mac proof comes from the two native hosted architectures.
6. **Stage specific files** (never `git add -A` — risks `nul`, `.env`, artifacts). **Commit** (`feat:`/`fix:`/`docs:`) with a `Co-Authored-By: Claude <model name> <noreply@anthropic.com>` trailer naming the model actually in use (e.g. `Claude Fable 5`). Open a PR (git-workflow); the PR runs `ci.yml` across macOS + Linux + Windows.
7. **Merge the PR to `main`.** `ci.yml` runs on the main push → `crates-publish.yml` then publishes the `nd300` crate (idempotent — a docs-only / unchanged-version merge safely skips).
8. **Push the release tag:** `git fetch origin main && git tag vX.Y.Z origin/main && git push origin vX.Y.Z`. This fires `release.yml` (6 targets + Global MSI + shell/PS installers + GitHub Release), which calls the tag/SHA-bound reusable `windows-installers.yml` (Corporate MSI + 2 Inno EXEs + sidecars) and then `macos-installer.yml` (direct PKG + compatibility DMG + sidecars). **Never reuse a tag** — cargo-dist treats tags as immutable; always go forward.
9. **Watch + verify:** watch `release.yml` and both reusable installer jobs. Confirm: crate published, exactly **32 assets**, accepted/stapled/Gatekeeper-valid direct PKG and compatibility DMG, byte identity, exact-SHA attestations, and `cargo install nd300 --force`.

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
5. **Add a CHANGELOG entry** for the patch (and a matching `HUMAN_CHANGELOG.md` entry under "Behind the scenes" — a one-liner that the previous release had a build problem is fine). One-line description of what was broken and that it's a build-only fix is fine — users don't need internal CI minutiae.
6. **Re-run the ship steps** (6–9 of the standard workflow): verify locally, commit, merge to `main` (→ crate), then push the **new** tag `vX.Y.Z` (→ release + installers). Verify the crate + all 32 assets after.
7. **Loop** if the patch run also fails. Bump again (`v3.0.2`, `v3.0.3`, ...) — never reuse a tag.

### Don't-do list (release safety)

- Never run `git push --force` or `--force-with-lease` to main during a release.
- Never re-tag an already-pushed version. Always go forward.
- Never `git add -A` — risks adding sensitive / unintended files.
- Never amend a commit that's already pushed (use a new commit).
- Never bypass `pre-commit` hooks (`--no-verify`) without explicit user authorization.
- Never delete a published tag or release artifact without explicit user authorization (downstream installers / package managers may already cache the URL).

## Changelog rule

This repo maintains two changelogs in parallel:

- `CHANGELOG.md` — the technical changelog (Keep a Changelog format). Version numbers, file paths, PR/issue links, metric thresholds, env var names, and code references all belong here.
- `HUMAN_CHANGELOG.md` — a plain-English companion. Every entry in `CHANGELOG.md` has a corresponding entry here, written for a non-engineer reader. No version numbers, no file paths, no jargon — just what changed and why it matters.

**When you update `CHANGELOG.md`, you MUST also update `HUMAN_CHANGELOG.md` in the same commit.** Skipping entries is not allowed; the two files must stay in lockstep.

### Translation rules

When writing the human-changelog entry, strip these out of the source entry:

- Version numbers (`3.0.4`, `v2.9.0`) — group by date or release name instead. For multiple releases on the same calendar day, either give each its own dated section or bundle them under one date with a descriptive subhead.
- File paths and module names (`src/actions/fix/`, `lib/cache/lru.go`).
- Function, class, struct, and variable names (`SharedCache::build_for_tech_mode()`, `TIMEOUT_QUICK`, `Action::apply`).
- Specific metric values ("reduced p99 from 340ms to 110ms" → "search is roughly three times faster now").
- Issue / PR / ticket / commit references (`#4821`, `JIRA-456`, SHAs).
- Env vars, registry keys, config flag names *unless* they're user-visible CLI flags (CLI flags like `--fix` and `--json` are fine to mention — they're how users actually drive the tool).
- Library / dependency version bumps unless user-facing — bundle them as "updated underlying libraries for security and stability."
- Jargon: *refactor*, *deprecate*, *polyfill*, *transpile*, *hydrate*, *idempotent*, *cfg-gated*, *fatal_environment_change*, *DAG*, etc. Pick an everyday phrase that conveys the same idea.

What to keep / add:

- What changed, in everyday words — a sentence is fine; a short paragraph is fine for big releases.
- Why it matters: the user-visible effect, or the reason the team made the change. If a change is purely internal with no user-visible effect, still record it under **Behind the scenes** — a sentence is fine.
- Plain category labels: **Added**, **Improved**, **Fixed**, **Removed**, **Security**, **Behind the scenes**, **Changed (be aware if you script against the tool)**.
- Honest about removals and breaking changes: say what stopped working and what to do instead. Keep CLI-flag mentions when the change actually affects user-typed commands.
- Tone: conversational but informative — imagine explaining the release to a non-technical teammate. No marketing fluff.

### Common ND300-specific translations

- `nd300 fix` / `nd300 -f` → "the auto-repair command" (or just "the fix command" — both are fine).
- `nd300 --update` / the cargo-install / installer split → "the built-in updater" or "the update command."
- "cargo install" / "cargo-dist" / "crates.io" → say it once if relevant ("the standard public Rust package registry," "the Rust package manager"), then refer to it plainly afterward.
- "WMI" → "the older Windows database for system info."
- "Triage loop" / "iteration" / "stage" / "action registry" → "rounds" / "steps" / "repair steps." The fix command now runs rounds of diagnose-then-fix-then-recheck, not fixed stages — describe it that way.
- "TCP slow-start," "trimean," "IQR," "inverse-variance merge," etc. → "the slow first few seconds of any test are properly excluded," "results are more resistant to misleading outliers," "providers with more consistent results contribute more to the final number."
- "WebSocket subprotocol header," "rustls-native-certs," etc. → "a missing piece of connection setup info" / "support for corporate certificate authorities."
- "Subcommand syntax" → "a new way to type the action commands."

### Verification before you commit

1. Open both files and confirm every release/section in `CHANGELOG.md` has a corresponding section in `HUMAN_CHANGELOG.md`.
2. Spot-check three random entries in `HUMAN_CHANGELOG.md`: do they avoid version numbers, file paths, function names, and jargon? Does each one say *what* changed and *why* it matters?
3. If a non-engineer skimmed the human entry, could they answer "what changed?" and "does this affect me?" If not, simplify further or add more context.

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
1. Run baseline diagnostics (the loop's probes force skip_speed — no action targets
   Speed, pinned by triage's no_action_targets_speed invariant test)
2. If everything passes → exit 0 ("nothing to fix")
3. EVIDENCE GATE (v3.4.0+): a failing baseline is re-confirmed with a second
   diagnostic pass. Iteration 1 plans against confirmed_failures (the
   intersection); failures seen in only one pass are recorded as intermittent
   (no repairs, no effectiveness credit). A transient blip → Fixed, zero actions.
4. Detect hard-block patterns (no link / ISP outage / enterprise VPN) → exit with guidance
5. Compute actionable failures, group by root cause, build_plan() returns ordered Vec<Action>
6. For each action in plan:
     - Medium-risk actions: require confirmation unless explicitly auto-confirmed with --yes
     - High-risk actions: render structured RiskExplanation prompt; require explicit 'y'
     - Apply, capture ActionOutcome, sleep stabilization window
     - If fatal_environment_change → break out of plan, re-probe immediately
7. Re-run diagnostics. If now passing → done. Else continue to next iteration.
8. Bounded by: ≤6 iterations, ≤4-min wall clock, per-action max_attempts
```

Effectiveness attribution credits a cleared failure only to the **most recent successful action that iteration whose declared `targets` contain it**; intermittent keys earn no credit. The loop body lives in `run_with_probe(probe: &mut impl DiagProbe, ...)` — the `DiagProbe` seam lets tests script diagnostic results through the real loop with zero subprocess IO (`ScriptedProbe` in `loop_runner.rs` tests).

### Module layout under `src/actions/fix/`

- **`action.rs`** — `Action`, `ActionId`, `Cost`, `Risk`, `Reversibility`, `RiskExplanation` types. The registry (`all_actions()`) returns every `Action` available on the current platform. `Action::apply(&self, config, restore: &RestoreRegistry)` dispatches to a free function per `ActionId` via match. Destructive `apply_*` fns (`apply_disable_consumer_vpns`, `apply_bounce_interface`, `apply_deep_stack_reset`) register their inverse op on the `RestoreRegistry` *before* mutating state; non-destructive ones ignore it. `Risk::High(RiskExplanation)` is a sealed enum variant: a high-risk Action *cannot* be constructed without a complete plain-language explanation.
- **`triage.rs`** — Pure planning. `actionable_failures()`, `group_by_root_cause()` (walks the hardcoded dependency DAG), `hard_block_detected()`, `build_plan()`. No IO. Unit-tested via `cargo test --lib actions::fix::triage`.
- **`session.rs`** — `Session` (per-run state: attempts, effectiveness, snapshots, action_log, plus `baseline_confirmation` + `intermittent` from the evidence gate — kept OUT of `snapshots` so JSON `iterations` math and the report's final-snapshot semantics hold) + `Reporter` (plain-language output for stdout) + `FinalOutcome` (now includes `Interrupted(Vec<DiagnosticKey>)` → exit 130) + the **`RestoreRegistry`** / `RestoreOp` / `restore_op` interrupt-safe restore machinery (see "Interrupt-safe restore" below). `WALL_CLOCK_CAP = 240s`.
- **`loop_runner.rs`** — races the caught-unwind loop against Ctrl-C plus SIGTERM/SIGHUP on Unix, then always drains restoration LIFO under a dynamic budget of 30 seconds per pending inverse plus 5 seconds before saving/reporting. Interrupt exits 130; panic drains then exits 101.
- **`report.rs`** — saves to the validated invoking user's `~/Downloads`, not effective root's home. It refuses unsafe report-directory state and atomically creates mode-`0600`, correctly owned reports; drain failures append "Manual recovery needed".
- **`stages.rs`** — macOS `DeepStackReset` is a fail-closed service off/on cycle, never service deletion/recreation. It maps exactly one enabled physical service, snapshots DNS servers + search domains, registers `ReEnableMacosService` before disable, cycles, and verifies enabled state plus expected route/reachability before resolving the restore token. Its production validation constant stays false until the ignored exact-production acceptance test passes on a disposable VM/service under root, exact interface/service acknowledgement, and an independent offline watchdog. Never run it on a host whose Codex/SSH control depends on that link. Forbidden-path tests prevent reintroducing service deletion, password capture, SSID scanning, or the incomplete snapshot. DHCP renew first proves DHCP mode; Manual/BootP/static/disabled/virtual/missing/ambiguous configurations are no-op.
- **`cmd.rs`** — bounded read-only helpers plus the owned mutation runner. Mutation children are serialized and killed/reaped before restoration on timeout/cancellation; paired down/up, off/on, release/renew, and stop/start supervisors always attempt the inverse. `run_cmd_capture` returns structured `CmdOutcome` report payloads.

### High-risk prompts

In interactive mode, every High-risk Action renders a multi-line block with `what`, `why`, side-effects, reversibility, and typical duration before running. Requires an **explicit `y`** — Enter alone, blank, or any non-`y` is treated as N. **`--yes` does NOT bypass these prompts** (by design — protects non-technicians from accidentally OK'ing destructive actions). In `--json` / non-interactive contexts, High-risk Actions are *skipped* (not auto-applied) with a clear marker in the report.

Medium-risk actions (VPN disable, DHCP reset, service restart, interface restart, and public-DNS changes) render a shorter confirmation prompt. `--yes` auto-confirms only these medium-risk prompts. JSON/non-interactive runs skip them when confirmation is required. Consumer VPNs are never disabled from JSON/non-interactive runs, and enterprise VPNs are never auto-disabled in any mode.

### Interrupt-safe restore (v3.0.9+)

`nd300 fix` is safe to cancel, time out, or crash mid-repair. The mechanism is a **restore registry + interrupt-safe drain**:

- **`RestoreRegistry`** (`session.rs`) is an `Arc<tokio::sync::Mutex<Vec<RegisteredOp>>>` (cheaply cloneable; **non-poisoning** mutex so a caught panic can't wedge cleanup). It threads through `Action::apply` into the destructive `apply_*` fns.
- **`RestoreOp`** is the inverse of a destructive change: `ReEnableInterface`, `ReEnableVpn`, `RestoreMacosDns`, and macOS-only `ReEnableMacosService { iface, service, dns_snapshot }`. No service-recreation restore exists.
- **Contract:** a destructive action calls `restore.register(op)` *before* mutating state (getting a token), then `restore.mark_resolved(token)` once it has restored the state itself on the normal path. Anything still unresolved at the end is replayed by `restore.drain()`.
- **`drain()`** runs pending ops newest-first, outside the registry lock, with a 30s per-op timeout. Its outer budget is `pending × 30s + 5s`, so every registered inverse receives one bounded attempt. It returns human-readable failures for any state it could not verify restored.
- **`run_and_finalize`** always drains after Ctrl-C, SIGTERM, SIGHUP, caught panic, timeout, or ordinary completion. Drain failures reach stdout/JSON/report.
- **Per-finding wiring:** every consumer VPN inverse is registered before disconnect; uncertain timeout/nonzero/readback leaves it pending, and positive provider/OS state readback is required to resolve it. Enterprise VPNs are untouched. Interface bounce and macOS service cycle register before mutation and resolve only after operational verification. `reverted: true` follows verification, never an attempted command.
- Every supervised mutation owns, serializes, terminates, and reaps its child before restoration. Paired mutations always attempt the inverse, preventing a late disconnect/down/stop/release from racing re-enable/up/start/renew.

### What to do when adding a new fix primitive

1. Add an `ActionId` variant in `action.rs`.
2. Add a free `apply_*` function for it (taking `&RestoreRegistry` if it mutates restorable state).
3. Add a match arm in `Action::apply`.
4. Append an `Action { ... }` entry in `all_actions()` declaring `targets`, `cost`, `risk`, `max_attempts`, `stabilization`. For `Risk::High`, attach a complete `RiskExplanation`.
5. Triage planning picks it up automatically — no changes needed in `triage.rs`.
6. **If the primitive makes a destructive, restorable change:** add a `RestoreOp` variant (cfg-gate it if it references platform-only types), a `restore_op` dispatch arm and a `label` arm (both exhaustive per platform), then `register` before mutating and `mark_resolved` after restoring.

Clean networks produce zero fix actions. Latency-only findings are advisory. DNS remediation remains staged. macOS deep reset never deletes/recreates a service; the gated replacement snapshots/restores exact DNS servers and search domains around one mapped physical service cycle and proves the expected route/reachability. Linux NetworkManager actions still operate on the active connection profile.

Enterprise VPNs are never auto-disabled. Reports go to the validated invoking user's `~/Downloads/nd300-fix-report-YYYYMMDD-HHMMSS.md`, never `/var/root` merely because `sudo` was used.

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
4. Prove exactly one channel: Cargo registry metadata, validated Unix archive receipt, exact macOS PKG receipt/payload/root-owned hash-bound pair, Windows standalone, or one registered MSI/EXE edition. Refuse unknown/conflicted/package-manager/local-build ownership.
5. Resolve latest once, then pin every internal artifact, sidecar, and verification to that exact tag; public entrypoints remain versionless.
6. Execute only the proven channel. Cargo stays Cargo and runs as the invoking user under a deadline; managed Unix archive stays archive; a macOS PKG downloads its exact-tag versionless direct PKG and opens Apple Installer; Windows stays exact standalone/MSI/EXE edition.
7. Verify both binaries. Unix archive download is bounded/size-limited, SHA-256 verified, and strictly two-file extracted. The macOS package route verifies the direct PKG identity, trusted timestamp, Gatekeeper policy, stapled ticket, exact payload/PackageInfo, and the installed receipt/hash/pair after Installer closes.
8. Transactionally replace/retire the pair and restore both on any failure. Never cross channels after failure.
9. Report the exact attempt, matching tagged installer when known, and versionless recovery URL while preserving the old owner.
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

- **Windows**: A running image cannot be overwritten but can be renamed in place. Writable installs retire both package binaries before strategy execution and restore them if every strategy fails. MSI uses transactional `MoveFiles` before `RemoveExistingProducts`; Inno disables Restart Manager shutdown and pairs `PrepareToInstall` retirement with `DeinitializeSetup` restoration; release automation fail-closed patches cargo-dist 0.31.0's generated PowerShell copy loop. The new binary's hidden retired-image cleanup removes or schedules deletion of only the versioned allowlisted siblings.
- **macOS/Linux**: `unix_install.rs` owns secure temp directories, archive/package verification, archive rollback markers, invoking-user Cargo, and origin classification. The package route requires exact `com.qubetx.nd300.pkg` metadata/payload, root-owned non-writable pair + hashes, Developer ID Installer identity/timestamp, a stapled ticket, and Gatekeeper; it opens the direct PKG, waits for Apple Installer, and verifies ownership afterward. Cancellation/mismatch never crosses channel. The release-only compatibility DMG is not part of new updater execution.
- **Unix uninstall**: Cargo → `cargo uninstall nd300`; ordinary invocation symlink → remove only the symlink; cargo-dist layout → require validated receipt; proven macOS PKG → authorize removal of only its pair/metadata and forget its receipt; package-manager/local-build/unknown → refuse. Location alone is not proof.
- **Cargo**: bare `cargo install` has no project post-install hook and cannot remove an unrelated registered product. Cargo updates remain Cargo-only and pin `--version =<resolved> --locked`; official installers can take over from Cargo, while a deliberate switch to bare Cargo should uninstall the registered channel first. Fresh Windows takeover remains same-scope and refuses opposite-scope owners before mutation.

### JSON Output

Supports backward-compatible `--json`. Stdout contains exactly one final JSON
object; transaction progress and child-process status belong on stderr. Every
result adds `install_channel`, `recovery_url`, and `requires_user_action`; a
failed known installer adds `exact_installer_url`. Windows retains
`install_origin`:
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

`macos-installer.yml` runs an internal-PR candidate and an exact-tag reusable
release path. Native Intel/ARM builds become one universal pair; both Mach-O
files are re-signed with pinned Developer ID Application identity/runtime/time,
then packaged with hash metadata as the preferred versionless
`nd300-universal-apple-darwin.pkg` using Developer ID Installer. That PKG is
notarized, stapled, Gatekeeper-checked, checksummed, and opened directly by new
updaters. The separately signed compatibility DMG contains byte-identical PKG
bytes for immutable v3.7.1/v3.7.2 clients. Hosted lifecycle tests on both
architectures cover container identity, standard archive takeover, same-version
repair, private higher-receipt downgrade, output/update selection, uninstall,
and reinstall. The private test fixture is never public. Tag publication refuses
overwrite, attests the PKG, DMG, and both sidecars to the tag SHA, and verifies
exactly 32 release assets.

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

Each installer writes `HKCU\Software\ND300\InstallSource = <marker value>`, but the marker is a hint rather than unconditional authority. Inno owns it through its Registry entry; MSI uses `install-maintenance` to write only on commit and clear on uninstall only if still current, avoiding shared component identity across the two products. `registered_install_owner()` scans both HKCU/HKLM uninstall views, requires expected display/scope/format, normalizes `InstallLocation` against the running binary, and reduces uninstall to an MSI product GUID independently proven by `MsiEnumRelatedProductsW` under a canonical UpgradeCode or a known Inno executable. A `.cargo\bin` path always wins; a unique proven owner wins over a contradictory marker; a marker is accepted only when its edition matches the path fallback. Update selects one matching installer, pins it and its sidecar to the resolved tag, verifies it, runs it, and re-checks the installed version.

**LOCKSTEP CONTRACT (do not break):** install paths, marker values, display names, MSI scope/UpgradeCodes, fixed Inno AppIds, versioned retired-image names, and hidden cleanup/maintenance arguments must stay aligned across all four installers, `update.rs`, `takeover.rs`, `migrate.rs`, `maintenance.rs`, and the cargo-dist PowerShell post-processor. All four package both binaries. Run the installer lockstep reviewer after any change.

**Updates preserve channel; fresh official installs take same-scope ownership.** Hidden `install-takeover` enumerates only proven owners and invokes Windows-Installer-proven MSI product codes/fixed Inno uninstallers without a shell. Global MSI↔EXE and Corporate MSI↔EXE changes are supported. Per-user ↔ per-machine changes are refused before mutation because Windows Installer cannot major-upgrade across contexts and elevated setup must not execute a user-writable uninstaller. `migrate-cleanup` remains the narrow Cargo/file cleanup; MSI retired-image cleanup uses commit execution so rollback material survives until `InstallFinalize` succeeds; Global MSI commit maintenance removes the exact legacy system-PATH entry and ensures the current entry once.

Top-level Windows uninstall uses the proven owner so both binaries, ARP registration, correct PATH hive, and marker leave together; raw registry command strings never go through a shell. Cargo/portable fallback retains the two-binary allowlist. JSON fields and strategy IDs remain compatible.

### `windows-installers.yml` — tag-triggered (consistent with TR-300)

ND-300's `Release` workflow fires on a `vX.Y.Z` **tag push**, so a tag-triggered Release run has `head_branch == the tag name`. `windows-installers.yml`:
- Is a reusable `workflow_call` invoked by the tag-push Release job with validated `tag` and `source_sha`; it no longer uses `workflow_run`, whose SHA is the default branch. Manual repair remains `workflow_dispatch` and must itself run at `--ref <tag>`.
- **Resolves the immutable tag**, requires it to match the caller/manual SHA, checks out that commit, and blocks Release announcement until success.
- **Pre-flight + idempotency:** probes the release for `dist-manifest.json` + `nd300-x86_64-pc-windows-msvc.msi` (torn-release guard); if all 6 corporate/EXE assets are already attached on a non-dispatch run, logs and exits 0. `workflow_dispatch` always rebuilds (`--clobber`).
- Keeps the WiX `candle`/`light -sice:ICE38 -sice:ICE64 -sice:ICE91` + Inno `iscc /DMyAppVersion=` mechanics. Windows completes 28 assets; the reusable macOS package job adds four for the final 32.

`Cargo.toml`: `allow-dirty = ["ci", "msi"]` (the `msi` is for the customized `wix/main.wxs` update, rollback, takeover, marker, and PATH maintenance), `/wix-corporate/**` + `/inno/*.iss` in the `include` list (never generated `inno/Output`), and the Windows-only `sha2`/`winreg` deps.

### Installer build pitfalls — validate locally before shipping

cargo-dist **suppresses** WiX `candle`'s real error (you only see "candle failed, exit 104"), so build all four installers locally before pushing. CI also runs permanent aarch64 GNU and x86_64 musl release-target compile gates. `.github/workflows/windows-origin-matrix.yml` builds candidate + fault installers once and tests the five update origins, pre-marker v2.9 Global MSI, four same-scope format changes, Cargo→Global MSI, and byte-identical opposite-scope refusal on fresh hosted VMs; dispatch `public` mode after release.
- **WiX:** `wix311-binaries.zip` → `candle -arch x64 "-dVersion=X" "-dCargoTargetBinDir=target\release" -ext WixUIExtension main.wxs` then `light -ext WixUIExtension` (corporate adds `-sice:ICE38 -sice:ICE64 -sice:ICE91`). **Quote `-d…` args** (PowerShell splits `3.2.0` on the dots otherwise).
- **Inno:** install Inno Setup 6 (`is.exe /CURRENTUSER /VERYSILENT`) → `iscc "/DMyAppVersion=X" inno\global.iss` (+ `corporate.iss`).
- **CNDL0104:** a WiX XML comment may NOT contain `--`. Never write CLI flag names (`--user-profile`) inside `<!-- … -->` — they're fine inside `ExeCommand=` attribute *values*.
- **Inno has no `{userprofile}` constant** — referencing it fails `iscc`; rely on the binary's process-env fallback.
- WiX cleanup CA uses `FileKey`/`ExeCommand` (type-18), NOT `WixQuietExec`: `cargo wix` + the bare `candle`/`light` build only link `WixUIExtension`, so a `WixQuietExec` CA fails to link.

### Cross-platform Rust gotcha

Shared `#[test]`s must use bare filenames / forward-slash paths; gate `C:\…` paths under `#[cfg(windows)]`. On Linux `\` is a literal char, so `Path::new(r"C:\x\nd300.exe").file_name()` returns the *whole* string — a `C:\`-path test passes on Windows but fails on Linux (this is exactly why the v3.2.0 allowlist test failed only on Linux CI). Windows-only helpers must be `#[cfg(windows)]` (not merely unused) or they trip `-D warnings` as dead code on macOS/Linux.

When pruning "unused" imports in files with per-OS `#[cfg]` branches (e.g. `actions/fix/stages.rs`), grep every platform's branch first — Windows-local clippy cannot see macOS/Linux usages (removing `use super::adapters;` failed 4 CI jobs in v3.4.0). Prefer platform-gating imports (`#[cfg(windows)] use ...`) over leaving them ungated.

### CI gates (`ci.yml` + `release.yml`)

`.github/workflows/ci.yml` runs fmt + clippy + test + release-build on **macOS + Linux + Windows** (with `RUSTFLAGS=-D warnings`) on every PR and main push, plus a `cargo-dist plan` check + cargo-audit — so cross-platform compile/test issues are caught at PR time, not at release. The cargo-dist `release.yml` remains the source of truth for the full 6-target matrix + the actual installer builds (which `ci.yml` does not exercise).

**cargo-dist is installed from the prebuilt binary tarball, not `installer.sh`.** `ci.yml`'s `dist-plan` and `release.yml`'s `plan` job fetch `cargo-dist-x86_64-unknown-linux-gnu.tar.xz` (`curl --retry`) and extract `dist` directly (it needs `chmod +x`), because GitHub's release CDN has repeatedly 504'd the small `cargo-dist-installer.sh` asset while the `.tar.xz` stayed up — and `dist-plan` is not `continue-on-error`, so that flake blocks the crates.io publish (it held up v3.3.1 for hours; fixed in v3.3.2). Don't revert these to `curl installer.sh | sh` (a `dist generate` would; re-apply the tarball install if so). Per-target `build-local-artifacts` jobs still use cargo-dist's platform-detecting installer (each runner needs a different host tarball).

### `nd300 dns` subcommand

`Nd300Command::Dns` (`src/cli.rs`) is the bare-subcommand form of `-d`/`--dns`. In `src/main.rs` it is routed through the **same semi-exit-early path** as the flag (sets a `run_dns` intent rather than the terminal subcommand block), so `nd300 dns` ≡ `nd300 --dns` (exit on failure, fall through to diagnostics on success). All other subcommands stay terminal.

## Speed Test Measurement Contract

Current runs use Methodology v5: `acquisition_v5.rs` records monotonic receiver-accounted byte counters and owns bounded resources; `measurement_v5.rs` estimates sustained throughput after explicit warm-up and repeatable ceilings; `engine_v5.rs` runs the common bounded Quick/Deep plan. Cloudflare and consenting M-Lab MSAK each contribute one primary-network estimate. NDT7 is a separate single-stream observation on M-Lab; supplementary providers cannot alter the headline. Ping is median idle HTTP RTT, with separately labeled loaded HTTP measurements and server TCP signals.

`METHODOLOGY.md` and `src/speedtest/measurement-contract-v5.json` are pinned to the public canonical TypeScript repository by `scripts/sync-v5-contract.mjs`. The independent Rust implementation and canonical TypeScript replay complete traces in `measurement-parity.yml`. Source/hash parity proves consistency, not real-device accuracy. Controlled reference comparisons, physical-device checks and animation overhead have separate acceptance evidence.

The older `statistics.rs`, `stat_primitives.rs`, provider implementations and `run_v4_reference` remain for regression comparisons. Their plateau, fastest-half upload, capacity/consensus, bootstrap and confidence-stop behavior must not enter the active v5 path or be presented as the meaning of new results. `docs/METHODOLOGY-v4.md` records the historical interpretation.

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
