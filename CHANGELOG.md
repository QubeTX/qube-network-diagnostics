# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [3.5.1] - 2026-07-06

### Fixed
- **M-Lab Locate 429s named honestly** — `ndt7.rs`/`msak.rs` discovery never
  checked the HTTP status, so a Locate rate limit surfaced as a misleading
  "missing results array" / parse error. A 429 now reads
  "discovery rate-limited: M-Lab Locate refused this network (too many
  requests) — try again later", matching the Cloudflare provider's existing
  rate-limit disclosure. Found by the three-repo M-Lab integration audit.
- **Cross-provider Locate short-circuit** — when NDT7's discovery is refused
  with a rate limit, MSAK no longer fires its own request into the same
  tripped per-IP limiter in the same run; it is recorded as skipped with the
  same honest reason.

## [3.5.0] - 2026-07-06

SpeedQX Methodology v4 — the unified cross-product measurement spec (see the new
`METHODOLOGY.md`, `methodology_version: "4.0"`), shipping alongside the speedqx.com
website (v3.0.x). Same algorithm, same constants, statistical parity asserted
bit-exactly by shared golden vectors.

### Added
- **Two new download-only providers (6 → 8 in speedqx):** **CacheFly**
  (`src/speedtest/cachefly.rs` — anycast CDN, 1/10/100 MB range-request ladder) and
  **Vultr** (`src/speedtest/vultr.rs` — 8 global POPs, min-RTT POP selection with
  per-probe timeouts so a dead POP can never stall the run).
- **`--fast` mode:** Cloudflare + NDT7 + MSAK with per-provider anytime-valid
  empirical-Bernstein confidence-sequence early stopping (strictly-predictable
  running mean; stop at half-width ≤ max(5% of estimate, 2 Mbps); disabled on
  high-RTT paths; a 25 s per-provider sampling cap that preserves completed work).
- **v4 statistical core** (`src/speedtest/stat_primitives.rs` + `statistics.rs`):
  pinned type-7 quantiles, PCG32 + Lemire deterministic PRNG, plateau warm-up
  detector (replaces the fixed 30% slow-start discard), circular block bootstrap
  (ℓ = max(2, round(∛n)), B = 2000) with BCa 95% intervals, Hodges–Lehmann
  cross-check, DerSimonian–Laird τ² + I², Hartung–Knapp–Sidik–Jonkman CIs
  (t-table pinned to df ≤ 7), and the **capacity + consensus hybrid merge**
  (capability priors CF/Apple/fast.com 1.0 · LibreSpeed/CacheFly/Vultr 0.95 ·
  MSAK 0.85 · NDT7 0.70; tier ≥ 0.85·max; 0.70 weight cap; MIN_MERGE_SAMPLES = 4
  with recorded exclusions).
- **Golden-vector parity suite** (`src/speedtest/golden_tests.rs` +
  `golden-vectors.json`, committed identically to the website/app repos): every
  fixture section asserted — arithmetic paths bit-exact (`f64::to_bits`),
  transcendental paths at 1e-9 relative — in both debug and release builds.
  Requires `serde_json`'s `float_roundtrip` feature (the default fast parser
  rounds fixture literals 1 ULP low).
- **New headline metrics** in results, `--json`, and the table renderer:
  capacity ± CI (headline) and consensus ± CI, agreement `{i2, band}`, PDV jitter
  (RFC 5481; RFC 3550 retained as a compat field), RPM responsiveness, and
  per-provider `availability` (`ran`/`failed`/`unavailable-platform`), plus
  `methodology_version` and `provider_set` stamps.

### Changed
- **Dense latency engine** on the HTTP providers (Cloudflare, LibreSpeed,
  fast.com, Apple): duration-scaled `clamp(50, round(dur×3.3), 200)` probes with
  a 3-probe warm-up discard at 50 ms pacing (up from ~10–20 probes discarding 2);
  min-RTT headline preserved, full percentile ladder collected; kernel
  `TCPInfo.MinRTT` cross-check from the M-Lab sockets unchanged.
- **Cloudflare downloads are now adaptively sized** (1 MB → 25 MB targeting ~2 s
  per request, mirroring the upload loop) instead of fixed 10 MB requests.
- Headline download/upload in the results table is now **capacity** (± CI margin)
  with consensus, agreement band, I², and RPM as quality rows; the per-provider
  breakdown gains availability; footer stamps `SQX methodology 4.0`.
- Warm-up probe discard on the new providers keeps all samples when a mostly
  failed probe run yields fewer than the discard count (mirrors the TS reference;
  the older providers' clamp-slice behavior is documented for later reconciliation).

### Fixed
- **`speed.cloudflare.com` per-IP rate limiting (HTTP 429) handled honestly:**
  large-payload throttling previously made the Cloudflare download spin for its
  whole phase collecting zero samples (and could silently understate merged
  results). Both transfer loops now stop on the first 429, disclose it in the
  provider's `error` (with the Retry-After horizon), keep the successfully
  measured direction, and let the merge exclude the throttled one.
- FAST mode's per-provider hard cap no longer discards a provider's completed
  measurements when the cap fires mid-phase on high-RTT paths.

## [3.4.0] - 2026-06-10

Comprehensive stability + accuracy hardening across all three subsystems
(diagnostics, speed test, fix flow), two new speed-test providers, and an
exhaustive technician mode (18 → 25 deep diagnostics).

### Added
- **Two new speed-test providers (4 → 6 in speedqx):**
  - **M-Lab MSAK** (`src/speedtest/msak.rs`) — the multi-stream `throughput1`
    successor to NDT7: two parallel WebSocket streams per direction measure
    aggregate capacity (NDT7's single flow structurally underestimates on
    high-BDP/lossy paths). Locate-API discovery, subprotocol
    `net.measurementlab.throughput.v1`, 500ms aggregate sampling, kernel
    `TCPInfo` MinRTT/RTT for ping/jitter. Same open M-Lab platform/policy as
    the existing NDT7 integration. Skip with `--skip-msak`.
  - **Apple networkQuality** (`src/speedtest/applenq.rs`) — the capacity half
    of the IETF responsiveness methodology against
    `mensura.cdn-apple.com`: 4 parallel HTTPS connections (large-object GETs /
    upload-sink POSTs) with 500ms aggregate sampling and 10-probe warmed
    latency. A fifth distinct CDN family — independent signal for the merge.
    Skip with `--skip-apple`. (Ookla Speedtest.net was evaluated and excluded:
    its EULA permits only personal, non-commercial use via the official CLI,
    with no third-party protocol authorization — rationale documented in
    `applenq.rs`.)
- **Bootstrap confidence intervals surfaced**: the previously dormant
  percentile-method bootstrap (`statistics::bootstrap_ci`) now runs on the
  pooled per-direction samples and displays as `941 Mbps ±12 Mbps` in speedqx
  and nd300 tech mode, with an additive `confidence_intervals` JSON field
  (suppressed below 8 pooled samples; deterministic via data-seeded PRNG).
- **Seven new deep diagnostics in technician mode** (18 → 25 total):
  `route_path` (bounded traceroute with first-hop/ISP-boundary/largest-jump
  analysis), `packet_loss` (sustained 30-probe bursts × 3 anycast targets),
  `nat` (double-NAT + CGNAT 100.64/10 detection), `dns_benchmark` (system vs
  public resolver timing + NXDOMAIN-wildcard hijack probe + subprocess-free
  DNSSEC validation check), `wifi` (structured signal/channel/band/PHY/
  security/rates via netsh/airport/nmcli-or-iw), `captive_portal`
  (redirect-disabled probe), `ntp` (dependency-free SNTP clock-offset check
  with HTTP-Date fallback; >30s skew is flagged — it breaks TLS).
- **Partial results when diagnostics hit the wall-clock cap**: `run_all` now
  enforces the cap internally and returns completed checks plus fabricated
  `timed_out` Fail rows for unfinished ones (exit code stays 2). JSON gains a
  top-level `timed_out` flag and per-row `timed_out` markers. **JSON
  consumers keying on the old `{"error":"timeout"}` shape must migrate.**
- New shared diagnostics infrastructure: `src/diagnostics/ping.rs` (one home
  for the ping invocation/parsing previously triplicated), `retry_probe` /
  `ping_budget` / `harvest_or` helpers and a `TRACE` (60s) timeout class in
  `diagnostics/util.rs`, and `src/speedtest/adaptive.rs` (throughput-targeted
  request sizing).

### Changed
- **Core diagnostic verdicts are de-flaked via consistent-failure requirements**
  (single transient blips no longer flip verdicts):
  - Gateway: 3-ping burst + conditional second burst — Fail now requires 0/6
    replies; partial loss is a new Warn with loss %. Additive
    `packets_sent`/`packets_received` JSON fields.
  - DNS: three independent domains (`dns.google`, `one.one.one.one`,
    `example.com`) probed concurrently with one retry each; verdict on
    count-resolved + **median** time. One slow domain among fast peers no
    longer Warns. Additive `resolution_tests` JSON field.
  - Latency: 6 pings/target (was 4); average-based Warns require ≥2 reachable
    targets; exactly one reachable target reports partial reachability
    instead of letting that target's latency set the verdict.
  - Ports: two endpoints per port on independent operators (80/443/53 across
    1.1.1.1↔8.8.8.8; 22 across github.com↔gitlab.com), 2 attempts each — open
    if either connects, so one provider's outage no longer Warns. New
    `outcome` field (`open`/`blocked`/`unresolved`); unresolved ports are
    excluded from the verdict denominator; `latency_ms` is now connect-only.
- **Speed-test merge hardening:** all samples flow through a non-finite
  sanitize choke point; unknown variance (1-sample providers, no-sample
  fallback values) is now treated as LEAST trusted in the inverse-variance
  merge (the old floor rule handed degenerate providers the highest weight);
  providers need ≥4 samples per direction to join the headline merge, with
  exclusions reported in an additive `merge_exclusions` JSON field and an
  `Excluded` display row. **Merged numbers shift where degenerate providers
  were previously over-weighted (intended).**
- **NDT7 sampling density:** 500ms per-interval download samples and
  server-`AppInfo`-delta upload samples (~20 per 10s iteration vs exactly 1
  before); upload frame growth switched to the reference-client
  16×-bytes-sent rule (1MB frames after ~8MB vs ~101MB). Per-provider
  `bandwidth_samples` arrays are substantially larger — flag for any JSON
  consumer parsing them.
- **Adaptive request sizing:** LibreSpeed download `ckSize` and the
  LibreSpeed/Cloudflare/fast.com upload payloads now target ~2s per request
  at the last measured throughput (LibreSpeed's fixed 100MB chunks yielded
  0–2 samples per 30s window on slower lines).
- **fast.com:** latency upgraded from one unloaded HEAD probe to a 10-probe
  warmed burst (min + RFC 3550 jitter — jitter was always null before);
  upload give-up softened to 5 consecutive failures with a small-payload
  retry. LibreSpeed server selection probes 30 candidates (was 5) and the
  hardcoded fallback list is now 9 geo-diverse servers (was 3 EU/US-East).
- **Real bufferbloat measurement** (tech mode): ping bursts run *during*
  sustained multi-stream download/upload saturation phases with per-direction
  A+–F grades (the old implementation admitted its loaded number was an
  estimate). Now honors `--fast`. Real path-MTU discovery (DF-bit binary
  search) joins the MTU section; IPv6 gains an HTTPS-over-v6 fetch and a
  v4-vs-v6 connect comparison; proxy gains PAC reachability + WPAD detection;
  ARP gains gateway-presence/duplicate-MAC health analysis.
- **`nd300 fix` evidence quality:**
  - The loop's diagnostic passes force-skip the speed test (no action targets
    Speed — pinned by a registry invariant test; a speed-only failure now
    reports cleanly instead of looping to `Exhausted`, and the freed budget
    funds the confirmation pass below).
  - A failing baseline is **re-confirmed with a second diagnostic pass**
    before the first repair plan; only failures present in both passes are
    acted on. Transient blips self-clear (outcome `Fixed`, zero actions);
    flickering failures are recorded as intermittent (report section + JSON
    `intermittent_failures`).
  - Effectiveness attribution fixed: a cleared failure is credited only to
    the most recent successful action that iteration whose declared targets
    contain it (previously every successful action got credit for every
    cleared key, letting natural recoveries bias planning).
  - Re-disabling a VPN after re-enable now requires two consecutive failed
    connectivity checks.
- `--speed-duration` default raised 10 → 15 seconds (the run_all cap formula
  is unchanged: 4×15+30 = the existing 90s floor); tech mode's cap gains a
  +150s deep-probe budget. speedqx's six providers run sequentially —
  a full default run is now ~5.5 minutes (use the skip flags to trim).

### Removed
- The legacy fixed Stage 1/2/3 orchestrators (`run_stage1/2/3`), their DNS
  fallback handlers, `wait_for_connectivity`/`verify_dns_stability`, and the
  `StepResult` type (~650 lines) — unused since the v3.0 triage loop. All
  platform primitives used by the action registry and restore machinery are
  kept; the captive-portal probe URL is now a shared named constant.

### Fixed
- A latent budget bug where multi-probe ping bursts could be truncated by the
  fixed 10s subprocess cap (budgets now scale with probe count via
  `ping_budget`).
- The aggregate latency fallback can no longer be set by a single noisy
  unloaded probe (jitter-bearing providers are preferred).
- A 21-way `tokio::join!` of inlined futures overflowed the Windows
  main-thread stack in tech mode; module futures are now boxed.

### Tests
- ~110 new unit tests: pure verdict functions for every reworked core module,
  per-OS canned-transcript parsers (ping/tracert/traceroute/netsh/airport/
  nmcli/iw/dig), merge/CI statistics, SNTP packet math, fix-loop scenarios
  via a new scripted-diagnostics seam (`DiagProbe`), and effectiveness
  attribution rules.

## [3.3.2] - 2026-06-08

Build/release-infra hardening only — no changes to the `nd300`/`speedqx` binaries.

### Fixed
- **cargo-dist install is no longer blocked by GitHub's flaky installer-script CDN.**
  `ci.yml`'s `dist-plan` job and `release.yml`'s `plan` job now fetch the prebuilt
  `dist` binary tarball (`cargo-dist-x86_64-unknown-linux-gnu.tar.xz`, with
  `curl --retry`) and extract `dist` directly, instead of piping
  `cargo-dist-installer.sh | sh`. GitHub's release CDN had repeatedly returned
  **504 on that small script asset** while the `.tar.xz` stayed available — and
  because `dist-plan` is not `continue-on-error`, it failed the whole CI run and
  blocked the crates.io publish (it held up the 3.3.1 release for hours). The
  per-target `build-local-artifacts` jobs still use cargo-dist's platform-detecting
  installer for their own runners (each needs a different host tarball).

## [3.3.1] - 2026-06-08

Dependency security pass — clears every RUSTSEC **vulnerability** flagged by the
CI `cargo-audit` job. No changes to the `nd300`/`speedqx` binaries' behavior.

### Security
- **`rustls-webpki` 0.103.9 → 0.103.13** (lockfile-only) clears four advisories in
  the real TLS path (`rustls`/`reqwest`/`tokio-tungstenite`): RUSTSEC-2026-0049
  (CRLs not treated as authoritative by Distribution Point), RUSTSEC-2026-0098
  (URI-name constraints incorrectly accepted), RUSTSEC-2026-0099 (wildcard-name
  constraints accepted), RUSTSEC-2026-0104 (reachable panic in CRL parsing).
- **`quinn-proto` 0.11.13 → 0.11.14** (lockfile-only) clears RUSTSEC-2026-0037
  (DoS in Quinn endpoints). `quinn` is reqwest's **optional `http3` stack, which
  ND-300 never enables or compiles** — the entry exists only because cargo-audit
  scans `Cargo.lock`, not the build graph — but it is bumped so the advisory clears.

### Changed
- **`indicatif` 0.17 → 0.18** — replaces the unmaintained `number_prefix`
  (RUSTSEC-2025-0119) with the maintained `unit-prefix`, clearing that warning.
  Pulls `console` 0.16; progress-bar output is visually identical (the used API —
  `ProgressBar`/`ProgressStyle` spinners and bars — is unchanged). Also refreshes
  the lockfile `rand` 0.9.2 → 0.9.4.

### Added
- **`.cargo/audit.toml`** documenting the two remaining **non-vulnerability**
  advisories that cannot be cleanly dropped, each with rationale (the CI audit job
  is now clean — 0 vulnerabilities, the two below ignored by ID):
  - **RUSTSEC-2024-0436** (`paste`, unmaintained) — a build-time proc-macro pulled
    transitively by `default-net 0.22`'s netlink stack (Linux default-gateway
    detection). No runtime risk; removable only by migrating off the now-renamed/
    abandoned `default-net` to its successor `netdev`, a cross-platform
    gateway-detection change out of scope here (tracked separately).
  - **RUSTSEC-2026-0097** (`rand`, unsound) — only manifests with a custom global
    logger calling `rand::rng()`, which ND-300 never does; the flagged `rand` is the
    never-compiled ghost behind reqwest's disabled `http3`/quinn, and no patched
    release clears it yet.

## [3.3.0] - 2026-06-08

Build + deploy cycle made consistent with the sibling TR-300 tool, plus a LICENSE
file. No changes to the `nd300`/`speedqx` binaries themselves.

### Added
- **`LICENSE` file** (PolyForm Noncommercial 1.0.0) at the repo root — matches the
  `Cargo.toml` `license` field and the sibling TR-300 repo (ND-300 previously
  declared the SPDX license but shipped no file). Added to the crate `include` list
  and shown in the Inno EXE installer wizards (`LicenseFile=..\LICENSE` in
  `inno/global.iss` + `inno/corporate.iss`, replacing the prior omission).
- **`.github/workflows/crates-publish.yml`** — publishes the `nd300` crate via
  `workflow_run` after `CI` succeeds on a `main` push (idempotent; skips an
  already-published version). Mirrors TR-300.

### Changed
- **Deploy model → tag-push, consistent with TR-300.** `release.yml` is now the
  stock cargo-dist **tag-triggered** workflow (+ the `nd-300-installer` legacy
  aliases); it no longer fires on a `main` push and no longer publishes the crate
  (that moved to `crates-publish.yml`, avoiding a double-publish). `windows-installers.yml`
  reverts to the tag-based filter (`startsWith(head_branch,'v')`, tag resolved from
  `head_branch`). **A release is now: bump version → merge to `main` (crate
  publishes) → push a `vX.Y.Z` tag (binaries/installers/GitHub Release).**
- Docs (`CLAUDE.md`, `AGENTS.md`): rewritten Release Process as the canonical
  tag-push deploy guide (incl. bumping `Cargo.toml` + the homepage version
  fallbacks), updated patch-bump loop, and the `windows-installers.yml` note.

## [3.2.1] - 2026-06-08

### Fixed
- **Windows EXE installer build (`inno/global.iss`).** The v3.2.0 consolidation work referenced a non-existent Inno Setup constant `{userprofile}` in the Global EXE's `[Run]` parameters, which failed the `iscc` step in `windows-installers.yml` — so the v3.2.0 GitHub release published without the Corporate MSI and the two Inno EXE add-ons (22 of 28 assets). Inno has no reliable *pre-elevation* user-profile constant (`{userprofile}` doesn't exist; `{%USERPROFILE}` / `runasoriginaluser` resolve to the admin account under separate-credential elevation), so the `--user-profile` argument is **dropped** from the Global EXE's `migrate-cleanup` invocation. `migrate-cleanup` falls back to the process environment (`CARGO_HOME`, then `%USERPROFILE%` / `%LocalAppData%`) — correct in the common case (an admin elevating their own session) and fail-safe otherwise (no copy under the admin profile → harmless no-op). The perMachine **MSI** still resolves the invoking user via its `Impersonate='yes'` custom action. Behavior is otherwise unchanged; v3.2.1 publishes the complete 28-asset Windows installer set.

## [3.2.0] - 2026-06-07

Cross-method install cleanup: consolidate to a single nd300/speedqx install
(one version, one edition) — interactively from the installers AND on a silent
self-update. Windows-only behavior; macOS/Linux already overwrite the single
`~/.cargo/bin` copy and are unaffected.

### Added
- **Hidden `nd300 migrate-cleanup` subcommand** (`src/actions/migrate.rs`, `#[command(hide = true)]` in `src/cli.rs`). Detects and removes duplicate installs by **reusing the existing tested deletion primitives** — it does *not* re-implement deletion:
  - From `src/actions/uninstall.rs`: `uninstall_path`, `CleanupReport`, `is_sole_package_in_dir`, and the `OUR_BINARIES` allowlist (now `pub(crate)`).
  - From `src/actions/update.rs`: `cargo_bin_dir`, `same_path`, `classify_install_path`, `current_install_shadows_cargo_install`, `current_exe_real_path`, `classify_shadow_cleanup`, `ShadowCleanupDecision`, `InstallOrigin` (now `pub(crate)`).
  - **Flags:** `--cargo-copy`, `--other-edition`, `--quiet`, `--dry-run`, `--json`, `--user-profile <path>`, `--cargo-home <path>`. With **no** target flag, defaults to `--cargo-copy` only.
  - **Targets:** the shadowing older `cargo install` copy in `~\.cargo\bin`, and the *other* Windows edition (Global perMachine `C:\Program Files\nd300\bin` ↔ Corporate perUser `%LocalAppData%\Programs\nd300\bin`). `--cargo-home`/`--user-profile` take precedence over `%USERPROFILE%`/process env so a perMachine installer can resolve the *invoking* user's `.cargo`.
  - **Hard safety guarantees (unit-tested):** only ever deletes `nd300.exe`/`speedqx.exe` (allowlist); never `cargo.exe`/`rustup.exe`/non-allowlisted files; never removes the `.cargo\bin` PATH entry (delegated to `uninstall_path`, which only edits PATH when `is_sole_package_in_dir` is true — and `.cargo\bin` always also holds cargo/rustup); never touches `~/Downloads`; never deletes the running install (`same_path` guard); never escalates privileges (a target needing admin we lack is reported `needs admin: <path>` and **continues** — exit 0); refuses shell-unsafe paths (via `uninstall_path`'s existing `build_delayed_delete_command` refusal).
  - **Exit 0 even on partial/empty/needs-admin** — cleanup is advisory and must never fail an installer. Only a true internal error (couldn't determine the running install's own location) is nonzero.
  - **Reporting:** human (removed / scheduled-on-exit / skipped:`<reason>` / needs-admin / failed, per target) and `--json` (stable `status` values `removed`/`scheduled`/`would_remove`/`skipped`/`needs_admin`/`failed`).
- **Installer consolidation UX + invocation on all four Windows installers.** Both options default ON, so a SILENT install consolidates too:
  - **Inno** (`inno/global.iss`, `inno/corporate.iss`): two default-checked `[Tasks]` (`cleancargo`, `cleanotheredition`) + a `[Run]` postinstall calling `{app}\bin\nd300.exe migrate-cleanup --quiet <flag>` with `Flags: runhidden waituntilterminated`, conditioned on each task. Added `AppMutex=ND300_Running` + `CloseApplications=yes`. Under `/SILENT` (the self-updater's EXE path) default-checked tasks fire automatically — both cleanups run with **no** `/MERGETASKS` suppression. The perMachine **Global** EXE passes the invoking user's profile via `--user-profile "{userprofile}"` (Inno `runasoriginaluser` is unreliable under right-click-Run-as-admin); the perUser **Corporate** EXE runs as the user (no `--user-profile`).
  - **WiX** (`wix/main.wxs` Global perMachine, `wix-corporate/corporate.wxs` Corporate perUser): `CLEANCARGO`/`CLEANOTHEREDITION` properties (default `1`, `Secure='yes'`) + a `ConsolidateDlg` WixUI dialog (two pre-checked checkboxes inserted between WelcomeDlg and CustomizeDlg) + two **deferred, `Impersonate='yes'`, `Return='ignore'`** custom actions scheduled `Before='InstallFinalize'`, conditioned `NOT Installed AND CLEAN…="1"` (install/upgrade only, not uninstall/repair). The CAs use `FileKey='exe0'` + `ExeCommand` (NOT `WixUtilExtension`'s `WixQuietExec`) because `cargo wix` / the bare `candle`+`light` build only link `WixUIExtension` — a `WixQuietExec` CA would fail the CI link step. `Impersonate='yes'` makes the perMachine MSI run cleanup as the invoking user, so the user's `.cargo`/`%LocalAppData%` resolve without needing `--user-profile`.

### Changed
- **Silent self-update now consolidates.** No code change was required in `src/actions/update.rs`: `try_msi_install` already runs `msiexec /i … /passive /norestart` (the default-`1` `CLEANCARGO`/`CLEANOTHEREDITION` properties stand), and `try_exe_install` already runs the EXE `/SILENT …` with no `/MERGETASKS` suppression (the default-checked tasks fire). Verified both paths leave the cleanups enabled.
- **`Cargo.toml`:** version `3.1.0` → `3.2.0`.
- Several `uninstall.rs` / `update.rs` items widened from private to `pub(crate)` so `migrate.rs` can reuse them (no behavior change to update/uninstall).

### Notes
- macOS/Linux compilation, and the WiX/Inno installer builds, are validated only in CI / on a Windows runner — the cross-platform Rust targets and `candle`/`light`/`iscc` are not run locally. All v3.1.0 update/uninstall behavior and tests are preserved.

## [3.1.0] - 2026-06-07

TR-300 parity release: full Windows distribution + self-update matrix, a `dns`
subcommand, and a batch of speed-test stability fixes.

### Added
- **Four first-class Windows installers** (MSI/EXE × Global/Corporate), each packaging **both** `nd300.exe` and `speedqx.exe`. A new `.github/workflows/windows-installers.yml` builds the Corporate MSI (`wix-corporate/corporate.wxs`, bare WiX 3 `candle`/`light` with `-sice:ICE38/64/91`), the Global EXE (`inno/global.iss`, perMachine/admin, system PATH), and the Corporate EXE (`inno/corporate.iss`, perUser/no-UAC, user PATH) via Inno Setup 6, plus a `<hex> *<name>` `.sha256` sidecar for each, and uploads all 6 add-on assets with `gh release upload --clobber`. The Global MSI continues to come from `wix/main.wxs` via cargo-dist. Corporate editions install to `%LocalAppData%\Programs\nd300\bin\` with no admin prompt; Global editions install to `C:\Program Files\nd300\bin\`. Pick one format per edition.
  - **Main-push trigger (divergence from TR-300):** ND-300's `Release` workflow fires on a `main` push and creates the tag itself, so `windows-installers.yml` triggers on `workflow_run: workflows:["Release"] types:[completed]` filtered to `conclusion=='success' && head_branch=='main'` (plus `workflow_dispatch` with a `tag` input that always runs). The tag is resolved by reading `version` from `Cargo.toml` at the triggering commit → `v$version`. A pre-flight probes the release for `dist-manifest.json` + the Global MSI, and **idempotently exits 0** when all 6 corporate/EXE assets are already attached (so no-op/docs main pushes don't rebuild).
- **`nd300 dns` subcommand** — bare-subcommand form of the existing `-d`/`--dns` flag (`src/cli.rs`). Routed through the same semi-exit-early path in `src/main.rs` (exit on failure, fall through to diagnostics on success), so `nd300 dns` ≡ `nd300 --dns`.

### Changed
- **Self-update parity with TR-300 (`src/actions/update.rs`).** ND-300's existing async-`reqwest` flow and richer cargo shadow-cleanup machinery (`ShadowCleanupDecision`, `classify_shadow_cleanup`, the uninstall integration + tests) are preserved; the following capabilities were added on top:
  - **Windows install-origin dispatch.** A `HKCU\Software\ND300\InstallSource` marker (written by each installer: `msi-global`/`msi-corporate`/`exe-global`/`exe-corporate`) is read by `read_install_source_marker()` (authoritative), with a path-based `classify_install_path()` fallback (`\program files\nd300\` → MsiGlobal, `\appdata\local\programs\nd300\` → MsiCorporate, `\.cargo\bin\` → CargoOrInstaller). `build_strategy_list` returns a **single** matching MSI/EXE strategy (no cross-fall-back between installer types). Four new `UpdateStrategy` variants (`MsiGlobal`/`MsiCorporate`/`ExeGlobal`/`ExeCorporate`) with stable `json_id`s.
  - **SHA-256-verified in-place installer upgrade.** `try_msi_install` / `try_exe_install` download the matching installer to `%TEMP%` via async reqwest, fetch the `.sha256` sidecar, `verify_checksum` (refuse-on-mismatch via the testable `checksum_verdict`), run `msiexec /i /passive /norestart` (handling exit 3010 reboot-required honestly) or the EXE `/SILENT /SUPPRESSMSGBOXES /NORESTART`, then re-exec `--version` to confirm the replacement landed.
  - **Prerelease-aware versioning.** `strip_prerelease_metadata` + an upgraded `is_newer` correctly handle `-rc`/`+meta` suffixes.
  - **`verify_cargo_post_install`** re-execs `--version` after `cargo install` (wired in **after** the shadow-cleanup success path); on a stale/crates.io-lag mismatch it falls through to the prebuilt installer instead of looping "update available."
  - **`rustup_update_stable_best_effort`** before `cargo install`; explicit GitHub **rate-limit (403)** messaging via `http_status_message`.
- **Self-update `--json` schema (Windows):** every update payload now carries a top-level `"install_origin"` field (`msi-global`/`msi-corporate`/`exe-global`/`exe-corporate`/`cargo-or-installer`/`unknown`). The field is Windows-only (omitted on macOS/Linux). New `"strategy"` values `msi_global`/`msi_corporate`/`exe_global`/`exe_corporate` join the existing set; `"method"` still maps to `cargo`/`installer`.
- **`Cargo.toml`:** version `3.0.11` → `3.1.0`; `allow-dirty = ["ci", "msi"]`; new `[target.'cfg(windows)'.dependencies]` `sha2 = "0.10"` + `winreg = "0.52"`; `/wix-corporate/**` and `/inno/**` added to the crate `include` list.

### Fixed
- **M1 — speedqx is now bounded by per-request timeouts and an outer wall-clock cap.** Each per-request builder in the Cloudflare, LibreSpeed, and fast.com download loops (and the fast.com upload loop) now sets `.timeout(remaining_budget(deadline))` (floored at 1s), so a stalled CDN socket can't block a single `resp.bytes().await` for the full 120s client timeout and overrun the deadline-based loop. `src/bin/speedqx.rs` now wraps the whole `speedtest::run` in a `tokio::time::timeout` outer cap (scaled to the configured durations, ≥120s) and exits 2 with a clear message on timeout (nd300 already had a wall-clock cap; speedqx had none).
- **M2 — `main.rs`'s overall `RUN_ALL_CAP` now scales with `--speed-duration`.** The fixed 90s cap is replaced by `max(90s, 4 * speed_duration + 30s)` (2 providers × 2 directions, plus headroom), so a deliberately long speed test (`--speed-duration 60`) isn't truncated and falsely reported as a "severely degraded network." `--fast` keeps the 90s floor.
- **L1 — honest per-provider failure.** Cloudflare, LibreSpeed, and fast.com now set `error = Some("no successful transfers")` when discovery/latency succeeded but **zero** transfer samples were collected, so the speedqx table shows an explicit error row instead of a silent N/A (and aggregation, which filters on `error.is_none()`, correctly excludes the provider).
- **L2 — speedqx honest exit code.** `speedqx` now exits non-zero when no provider produced positive throughput in either direction, mirroring `diagnostics/speed.rs::determine_speed_status`'s "measured" check. A slow-but-working link (positive throughput) still exits 0.
- **L4 — NDT7 keeps earlier samples on a mid-run error.** A mid-run iteration error in the NDT7 download/upload deadline loops now `break`s (retaining samples already collected) instead of `?`-collapsing the whole provider to an error. A first-iteration failure that collected nothing still reports an honest `"no successful transfers"` provider error.

### Known limitations
- L3 (the cosmetic `Risk::Low` label on `RestartNetworkServices`) and L5 (fast.com token-scraping brittleness) are intentionally **not** addressed in this release — documented as known/low.

## [3.0.11] - 2026-06-07

### Fixed
- **M5 — a completely failed speed test now reports failure (exit 2) instead of a misleading "too slow" warning.** When every speed-test provider errors out (or returns zero throughput), `diagnostics/speed.rs` previously fell through `determine_speed_status` to the slow-link branch, producing a `0.00 Mbps down / up` aggregate, a "Download too slow for most activities" warning, and exit 1. `determine_speed_status` now first checks whether *any* provider succeeded (no `error`) **and** reported positive throughput in either direction; if not, it returns the new `SpeedStatus::Failed`, which `check()` maps to a `Fail`-status diagnostic with an honest standalone summary ("Speed test failed — no provider returned a result", with no fabricated 0.00 Mbps line). A genuinely slow-but-working link (e.g. 0.3 Mbps from a successful provider) is unaffected — the positive-throughput guard keeps it `Poor` → Warn.
- **L1 — the NDT7 speed test's per-direction safety timeout now actually fires.** The NDT7 download loop (`speedtest/ndt7.rs`) re-armed a fresh 30s `tokio::time::sleep(SINGLE_TEST_TIMEOUT)` on every `tokio::select!` iteration, so the safety cap could never elapse and was dead code. It is now a single pinned absolute deadline (`Instant::from_std(start + SINGLE_TEST_TIMEOUT)`) computed once before the loop and driven with `sleep_until`, mirroring the existing per-session `deadline` arm. The upload loop, which previously had **no** safety cap at all, gets a symmetric pinned cap so a wedged `write.send` is now bounded.

### Changed (be aware if you script against the tool)
- The exit code for a fully-failed speed test (every provider errored / zero throughput) is now **2** (Fail) instead of **1** (Warn), and the `Speed` diagnostic's summary is `Speed test failed — no provider returned a result` rather than a `0.00 Mbps … too slow` line. A slow-but-working connection is unchanged (still Warn / exit 1).

### Behind the scenes
- **L5 — system-info lookups in the core diagnostics no longer briefly block the async runtime.** The synchronous, potentially-blocking `sysinfo::Networks::new_with_refreshed_list()` and `default_net::get_default_gateway()` calls in `diagnostics/gateway.rs`, `diagnostics/public_ip.rs`, `diagnostics/interfaces.rs`, and `diagnostics/shared_cache.rs` are now wrapped in `tokio::task::spawn_blocking`, returning owned `Send` data with a `JoinError` fallback to the pre-existing empty/`None` result (never an `unwrap` panic). `interfaces.rs` extracts owned per-interface fields (name, MAC bytes, IPs, rx/tx) inside the blocking closure and keeps its async, timeout-wrapped `get_wifi_summary()` call outside it; the `is_up` semantics (`has an IP && rx > 0`) are byte-for-byte preserved. Mirrors the existing `spawn_blocking` pattern in `actions/fix/adapters.rs` and `diagnostics/vpn.rs`.

### Known limitations
- **L2 (deferred) — localized (non-English) Windows.** On non-English Windows display languages, some deep-diagnostic modules that parse localized `netsh` / `ipconfig` / `netstat` output may return empty or partial results, because they match English labels and decode console output as UTF-8. This is tracked for a future locale-invariant parsing pass and is not addressed in this release.

## [3.0.10] - 2026-06-07

### Fixed
- **M1 — `cargo install nd300` can no longer fail on a read-only / hardened / sandboxed build cache.** `build.rs` previously wrote the generated man pages into `CARGO_MANIFEST_DIR/man/` with `.unwrap()`'d `fs::create_dir_all` + `fs::write` calls. On the primary install path (`cargo install nd300`), that directory is the registry extraction under `…/registry/src/…`, which is read-only on locked-down machines — the failing write would panic the build script and abort the install. `build.rs` now (a) renders both man pages into in-memory buffers (the only step that keeps its panic, since a failed render is a genuine bug), (b) writes `OUT_DIR/man/*.1` strictly best-effort (`create_dir_all(...).is_ok()` guard + `let _ = fs::write(...)`), and (c) writes the committed repo `man/*.1` only when a new `is_dev_source_tree(CARGO_MANIFEST_DIR)` check confirms the build is happening in a live source checkout (i.e. the manifest path is **not** under `/registry/src/`, `/registry/cache/`, `/git/checkouts/`, or `/vendor/`), and even then best-effort. Net: `cargo install` can never panic on a man-page write, while a normal local `cargo build` still refreshes the committed `man/*.1` (so the dev tree stays in sync). Man pages still ship in release archives and the published crate via `Cargo.toml`'s `include = ["man/**"]`; nothing reads `OUT_DIR/man`.
- **H5 — Windows uninstall/update no longer reports binary removal before it actually happens (and no longer defeats the updater's shadow guard).** On Windows the running `.exe` can't be deleted in place, so the uninstaller spawns a background `cmd /C … del` that removes it after the process exits. Previously `uninstall_path` set `binary_removed = true` immediately on spawn — a false "removed" that, because the updater reuses `uninstall_path` for shadow-cleanup (`actions/update.rs`), silently bypassed the updater's Fatal "old install still shadows the new cargo binary" guard. `CleanupReport` now carries a distinct `binary_removal_scheduled` flag: the Windows branch sets it (and leaves `binary_removed = false`) on a successful spawn. `uninstall` reports "scheduled for removal (completes when this process exits)" and treats `binary_removed || binary_removal_scheduled` as success. The updater's `cleanup_shadowing_current_install_after_cargo_success` now returns Ok **with an honest warning** when removal was only scheduled (the new cargo binary wins in a new shell; manual cleanup instructions are printed if it doesn't), instead of either a silent success or a false Fatal. `cleanup_current_install_for_cargo_retry` stays Fatal when the file isn't actually gone — an immediate cargo retry needs it removed now — but now emits explicit "close this nd300 process and re-run `nd300 update`" instructions for the scheduled case.
- **L3 — Windows uninstall now fully cleans the PATH entry even with a trailing slash.** `remove_from_user_path` now compares each PATH entry against the install dir ignoring case **and** a trailing `\`/`/` (via a new pure `path_entry_matches_target` predicate, mirroring `same_path` in `actions/update.rs`), so an entry like `C:\…\bin\` is removed instead of left behind as a dead PATH stub. Only the comparison is normalized; retained entries keep their original spelling.
- **L4 — the Windows delayed-delete command is now hardened against shell metacharacters.** The `cmd /C … del "<path>"` string is built by a new `build_delayed_delete_command` helper that refuses (returns `None`, reported as a failure with an explanatory note) any path containing `"`, a newline/carriage-return, or a `cmd` metacharacter (`& ^ | < >`) — all of which are already illegal in real Windows paths — and escapes `%` to `%%` so `cmd`'s environment-variable expansion can't mangle a path with a literal `%`.

### Changed (be aware if you script against the tool)
- `nd300 uninstall --json` gains an additive boolean field `binary_removal_scheduled`. The existing `binary_removed` field keeps its literal "is the binary gone right now?" meaning (so it now reads `false` on a successful Windows uninstall, where deletion completes on process exit), while `success` and the exit code key off `binary_removed || binary_removal_scheduled`. The field is purely additive and non-breaking on macOS/Linux, where it is always `false` and `binary_removed` is unchanged.

## [3.0.9] - 2026-06-07

### Added
- **`nd300 fix` is now safe to cancel, time out, or crash mid-repair.** A new restore registry (`actions/fix/session.rs`) records the inverse of every destructive change before it is applied, and `loop_runner::run_and_finalize` now races the triage loop against `Ctrl-C` (`tokio::signal::ctrl_c`) and wraps it in `catch_unwind`. On *every* terminal path — normal completion, user-declined, wall-clock cap, Ctrl-C, or a caught panic — the registry is drained (bounded by a 90s outer cap plus a 30s per-op cap) so any half-applied network state is rolled back. Ctrl-C now exits 130 via the new `FinalOutcome::Interrupted`; a caught panic restores state first, then re-raises as exit 101. The registry uses a non-poisoning `tokio::sync::Mutex` so a panic mid-loop never wedges cleanup.

### Fixed
- **H2 — Consumer VPNs are now re-enabled instead of being left off silently.** `apply_disable_consumer_vpns` (`actions/fix/action.rs`) registers a `ReEnableVpn` restore op per disabled VPN before reporting success and wires up the previously-dead `vpn::offer_reenable` (now reachable). If the run is interrupted before the offer, the drain reconnects each still-registered VPN. Enterprise VPNs are still never auto-disabled, and JSON/non-interactive runs still skip VPN disabling entirely.
- **H3 — A macOS deep reset can no longer strand the Mac with no network service.** `stage3_macos` (`actions/fix/stages.rs`) registers a `RecreateMacosService` restore op (capturing iface/service/snapshot) before `-removenetworkservice`. Recreate now goes through a shared `recreate_and_restore_macos_service` helper that retries the create once and then restores the captured DNS/proxy/service-order/IPv4/IPv6 snapshot. If recreate still fails, the op is left registered (so the terminal drain retries it) and a manual-recovery message is returned, instead of returning early and skipping the snapshot restore.
- **H4 — An interface bounce no longer leaves the adapter disabled if re-enable fails.** `apply_bounce_interface` registers a `ReEnableInterface` op before disabling and re-enables with a retry-once (porting the legacy Stage 2 2s-wait retry). If both attempts fail, the op stays registered for the drain and the action returns a loud, actionable message including the exact platform command to bring the adapter back up.
- **M3 — Linux `stage3_linux` no longer claims "Deleted profile" when the delete failed.** The result of `nmcli connection delete` is now checked; on failure the function returns an error with the `nmcli` stderr instead of unconditionally recreating on top of the still-present profile.

### Changed (be aware if you script against the tool)
- `nd300 fix --json` output gains two fields: `interrupted` (bool) and `manual_recovery_needed` (array of human-readable strings for anything the drain could not restore). A new `outcome` value `"interrupted"` is emitted on Ctrl-C / caught panic, with `exit_code` 130.
- The fix report (`~/Downloads/nd300-fix-report-*.md`) gains a prominent "⚠️ Manual recovery needed" section listing any restore the drain could not complete.

### Behind the scenes
- The pre-existing `diagnostics/util.rs` unit test `nanosecond_resolve_times_out_to_none` (added in 3.0.8) is timing-dependent and can flake on fast machines when the resolver answers within the 1-nanosecond budget; it was left unchanged here to keep this release scoped to the fix flow, and verified to be a pre-existing flake on the prior release rather than a Phase 2 regression.

## [3.0.8] - 2026-06-07

### Fixed
- **Diagnostics no longer hang on a broken network.** Every diagnostic subprocess call (`ping`, `netsh`, `ipconfig`, `scutil`, `nmcli`, `ip`, `arp`, `route`, `netstat`, `dig`, `nslookup`, `resolvectl`, `wg`, etc.) and both DNS resolver calls (`diagnostics/dns.rs` resolution test, `diagnostics/ports.rs` port test) are now wrapped in explicit timeouts via a new `diagnostics/util.rs` helper (`run_with_timeout`, `lookup_host_timeout`). Budgets: 5s for DNS resolution, 10s for network-touching probes (`ping`/`nslookup`/`dig`/`resolvectl`), 5s for local state queries. On timeout the call returns the same "unreachable/empty" result it already returned on spawn failure, so behavior on a healthy network is unchanged. Previously a black-holed resolver or wedged subprocess could stall `nd300` (and `nd300 fix`, which re-runs diagnostics) for the OS resolver's full timeout.
- `main.rs` now wraps the top-level `diagnostics::run_all` in an overall 90-second wall-clock cap and a `Ctrl-C` handler. Ctrl-C prints `Interrupted.` and exits 130 (`{"error":"interrupted","interrupted":true}` in `--json`); the wall-clock timeout prints a degraded-network message and exits 2 (`{"error":"timeout","timed_out":true}` in `--json`).

### Changed
- **Latency check now pings its three targets concurrently** instead of sequentially (`diagnostics/latency.rs`), via `futures_util::future::join_all`. Worst-case latency-phase time on a dead network drops from ~24s to ~10s while the reachable/average math is byte-identical (order is preserved). The `ping` inside `ping_multiple` is also bounded by the 10s subprocess budget.

### Build
- Resolved pre-existing Clippy lints surfaced by the current toolchain (`unnecessary_map_or`, `manual_split_once`, `nonminimal_bool`, `ptr_arg`) in `diagnostics/adapters.rs`, `diagnostics/dns_cache.rs`, and `diagnostics/listening_ports.rs` so `cargo clippy --all-targets --all-features -- -D warnings` passes cleanly. Behavior is unchanged; these are mechanical, lint-only rewrites.

## [3.0.7] - 2026-05-11

### Documentation
- Republished the already-correct local README and documentation to crates.io so the registry page reflects the complete release/update workflow, including Cargo-first updates, non-Cargo install cleanup, legacy installer aliases, and the canonical `cargo install nd300` path.
- Aligned local agent docs, `CODEX_PROJECT.md`, and the original project plan with the current release/update workflow. Runtime behavior is unchanged from `3.0.6`.

## [3.0.6] - 2026-05-11

### Fixed
- `nd300 update` now handles Cargo installs over older non-Cargo installs more safely. If the cargo strategy succeeds while the currently running `nd300`/`speedqx` copy lives outside Cargo's bin directory, the updater removes that old installer-managed layout so the newly installed Cargo binary is not shadowed on `PATH`.
- If `cargo install nd300 --force` reports an existing `nd300` or `speedqx` binary collision, the updater now runs ND300's uninstall cleanup for the current install and retries the Cargo install before falling back to GitHub installers.

### Documentation
- Clarified that bare `cargo install nd300` cannot run project-specific uninstall hooks for unrelated old install locations; users with a shadowing legacy install should run `nd300 update` or `nd300 uninstall` first so ND300 can clean up the old layout.

## [3.0.5] - 2026-05-11

### Documentation
- Expanded release notes so the latest crates.io package and GitHub release explicitly document the complete ND300 publish path: main-branch source checks, full cargo-dist builds, GitHub release/updater asset hosting, legacy installer aliases, and final crates.io publishing through `CARGO_REGISTRY_TOKEN`.
- Documented all supported install and update paths in the release notes: `cargo install nd300`, GitHub shell installer `nd300-installer.sh`, PowerShell installer `nd300-installer.ps1`, Windows MSI/archives from GitHub Releases, and legacy `nd-300-installer.*` aliases for older self-updaters.
- Aligned local `CLAUDE.md` and the global Codex `AGENTS.md` with the main-branch ND300 release workflow so future agents use GitHub Actions as the deploy path instead of manual local publishing.

### Changed
- Bumped package and release metadata to `3.0.5` so the corrected changelog and release workflow documentation are included in the latest crates.io artifact and GitHub release. Runtime behavior is unchanged from `3.0.4`.

## [3.0.4] - 2026-05-11

### Added
- First crates.io publication under the canonical package name `nd300`, owned by the current maintainer account, with the natural install command `cargo install nd300`.
- Publish automation for crates.io and GitHub releases from `main`: every push now runs format, tests, clippy, a crates.io dry-run, the full cargo-dist build matrix, GitHub release publishing, updater asset upload, and finally crates.io publishing when the `Cargo.toml` version is new.
- GitHub release assets now use the package-aligned cargo-dist names `nd300-installer.sh` and `nd300-installer.ps1` for the standard updater/install path.
- Legacy installer aliases (`nd-300-installer.sh` and `nd-300-installer.ps1`) are uploaded with new releases so older installed copies can still self-update through their existing hardcoded URLs.

### Changed
- The crates.io package name is now `nd300`, while the library target remains `nd_300`, binary names remain `nd300` and `speedqx`, and product branding remains ND300.
- README installation instructions now advertise the supported public paths: shell installer, PowerShell installer, Windows MSI, `cargo install nd300`, and source build.
- `nd300 update` / `speedqx update` now use `cargo install nd300 --force` when cargo is available, then fall back to the new `nd300-installer.*` cargo-dist assets from GitHub Releases.
- Windows MSI display/install directory naming now follows the new `nd300` package slug instead of `nd-300`.

### Fixed
- CI Clippy compatibility with the current GitHub runner toolchain.

## [3.0.3] - 2026-05-10

### Fixed
- `nd300 fix` is now evidence-staged instead of status-only for common healthy or near-healthy networks: clean networks produce no actions, latency-only findings are advisory, DNS failures start with cache flush, and public DNS is considered only after safer DNS-specific repair has failed.
- `--yes` now reaches the runtime `Config`, so scripted runs can auto-confirm medium-risk repair actions while high-risk actions still require an explicit interactive approval.
- Non-interactive and JSON fix runs no longer disable consumer VPNs, because they cannot safely guide re-enable/recovery steps.
- macOS deep network-service reset now snapshots and restores DNS servers, proxy settings, service order, and IPv4/IPv6 mode instead of unconditionally forcing DHCP plus Cloudflare DNS after service recreation.
- Linux NetworkManager DNS changes now resolve the active connection profile for the interface instead of assuming the profile name equals the device name, and Wi-Fi profile reset collects reconnection details before deleting the old profile.
- macOS latency diagnostics now use the correct `ping -W 2000` millisecond timeout instead of `-W 2`, which caused healthy endpoints to be reported as unreachable.
- Speed-test aggregation now merges all successful providers with a bounded N-provider inverse-variance merge, detects divergence across the full provider spread, ignores failed/non-2xx transfer samples, and makes NDT7 respect short requested durations.
- `--fast` and `--speed-duration` now work both before and after subcommands, including `nd300 fix --fast`.

### Added
- Regression tests for clean/no-op and staged DNS planning behavior, medium-risk confirmation rules, JSON-mode VPN safety, macOS network snapshot restore command generation, and all-provider speed aggregation/divergence.

## [3.0.2] - 2026-05-07

### Fixed
- **`nd300 update` no longer crashes with `Failed to run cargo install: No such file or directory (os error 2)`** when the binary lives under `~/.cargo/bin/` but no Rust toolchain is installed. The previous detection assumed any binary path containing `.cargo` + `bin` meant "user installed via `cargo install`," but the cargo-dist shell installer also defaults to that location whenever `CARGO_HOME` or rustup is present — so users who installed via the official shell installer were misclassified and the updater tried to spawn `cargo`, which doesn't exist on their system. Bug has shipped silently since v2.9.0.

### Changed
- **Update logic is now a probe-and-retry chain** instead of a single dispatched method. Cargo first when `cargo --version` succeeds, cargo-dist installer as universal fallback (curl → wget on macOS/Linux, `powershell.exe` → `pwsh.exe` on Windows). When cargo isn't invokable it's pruned from the candidate list entirely, so the ENOENT crash can't recur. Failures fall through to the next candidate; the chain only stops on first success. When every strategy fails, both pretty and `--json` output now show a per-attempt diagnostic block listing what was tried and why each failed.
- Hardened the installer pipelines: `set -e -o pipefail` and `curl -fLsS` on Unix (a 404 or DNS error now fails loudly instead of being silently swallowed by `sh` exiting 0 on empty input); `$ErrorActionPreference='Stop'` plus `-NoProfile -NonInteractive` on Windows. These are latent silent-success bugs in the same code path, fixed alongside the main issue.
- `--json` output gains a `"strategy"` field (precise variant) and an `"attempts"` array on failure. `"method"` continues to map to `"cargo"` or `"installer"` for backward compatibility.

## [3.0.1] - 2026-05-07

### Fixed
- macOS release build broken in v3.0.0 — `stages::detect_macos_service` was private but the new triage-loop action registry calls it. Marked `pub` on the macOS cfg path so cross-platform builds succeed. Functionality unchanged on Windows / Linux; this is a build-only fix.

## [3.0.0] - 2026-05-07

### Added
- **Subcommand syntax for action commands** — `nd300 fix`, `nd300 update`, `nd300 clear-dns`, `nd300 uninstall` now work in addition to the legacy flag forms (`nd300 -f`, `nd300 --update`, `nd300 --clear-dns`, `nd300 --uninstall`). The legacy flags continue to work — both forms produce identical behavior. `speedqx update` mirrors the same pattern alongside `speedqx --update`.
- **Diagnostic-driven triage loop in `nd300 fix`** (replaces the fixed three-stage script): tests the network, identifies which specific diagnostics failed, applies only the fix actions that target those failures, re-tests, and repeats until everything passes or no further actions are available. Bounded by three independent caps (≤6 iterations, ≤4 minute wall clock, per-action `max_attempts`) — the loop always terminates.
- **Causal grouping in triage** — when a failure cluster (e.g. interface down → adapters/gateway/DNS/public-IP all fail) traces back to a common ancestor in the dependency DAG, the loop fixes the ancestor first instead of wasting steps on cascading symptoms.
- **High-risk action prompts** — destructive actions (Winsock reset, recreate macOS network service, delete NetworkManager profile) now render a structured plain-language explanation block before running: what the action does, why it's being attempted right now, what the user will experience, reversibility, typical duration. **An explicit `y` is required** to proceed — Enter alone, blank input, or anything else is treated as No. **`--yes` does not bypass these prompts** — they always require interactive confirmation, by design. Written for both technical and non-technical users.
- **Hard-block detection** — the loop short-circuits cleanly with guidance (instead of thrashing) for failure shapes that can't be auto-fixed: no physical link, ISP-side outage, active enterprise VPN.
- **Effectiveness scoring within a session** — when two candidate actions both target the same failure, the one that has already shown improvement is preferred.
- **Per-action stabilization windows** — every Action declares how long the system needs to settle after it runs before the next diagnostic re-probe (typically 1.5–5 seconds depending on action type — DNS flush is fast, interface restart is slow). The loop sleeps that window before re-testing instead of using a global one-size delay.
- **Fatal-environment-change re-probe** — actions that rebuild adapter / interface state (DHCP renew, adapter restart, NetworkManager profile rebuild, macOS network service recreation) mark themselves as `fatal_environment_change`. When one runs, the loop breaks out of its current plan and re-runs diagnostics immediately rather than continuing through actions selected against the now-stale failure set.
- **OS-aware action registry** — `all_actions()` only returns actions valid for the host platform via `#[cfg]`-gated branches, so Windows-specific actions (Winsock reset) never enter a macOS plan and vice versa. Adding a new fix primitive is a 5-step process documented in `CLAUDE.md`.
- **Comprehensive Markdown reports** at `~/Downloads/nd300-fix-report-<timestamp>.md` containing: plain-summary verdict, baseline diagnostics snapshot, per-iteration timeline (action + captured stdout/stderr/exit/duration + diagnostic delta), final snapshot, environment block, and a "what to try next" section keyed off the remaining failure set.
- **`-y` / `--yes` global flag** — auto-confirms Medium-cost prompts in CI / scripted use. Reserved for future Medium-risk confirmations; explicitly does **not** auto-accept High-risk prompts.
- New module layout under `src/actions/fix/`: `action.rs` (Action types + registry), `triage.rs` (pure planning logic, fully unit-tested), `session.rs` (state + plain-language Reporter), `loop_runner.rs` (loop driver). `report.rs` rewritten for the new iteration-oriented timeline.
- New `CmdOutcome` capturing struct in `cmd.rs` with `run_cmd_capture()` for richer subprocess forensics in reports.

### Changed
- **`nd300 -f` / `nd300 fix` no longer follow a fixed Stage 1 → Stage 2 → Stage 3 sequence.** It now runs diagnostics first and applies only the actions that target the actual failures.
- A clean network completes `nd300 fix` in under 8 seconds and applies zero actions (was previously a full Stage 1 sequence regardless).
- `--json` output schema for the fix action is restructured around the new triage loop (outcome, iterations, applied_actions, remaining_failures, hard_block, etc.). Existing scripts that read `stages[]` and `resolved_at_stage` will need updating.
- Output flags (`--json`, `--ascii`, `--no-color`, `--verbose`) marked global so they work both before and after a subcommand: `nd300 fix --json` and `nd300 --json fix` both work.

### Backwards compatibility
- Every existing flag still works exactly as before: `-f`, `--fix`, `--update`, `--clear-dns`, `--uninstall`, `-t`, `-T`, `--fast`, `--json`, `--ascii`, `--no-color`, `--verbose`, `--speed-duration`, `-d`, `--dns`. The subcommand form is purely additive.

### Migration notes
- If you parsed the old fix `--json` output's `stages[]` array, switch to `applied_actions[]` (one entry per action attempted, with `iteration`, `action`, `label`, `ok`, `message`, `duration_ms`).
- If you depended on the fixed Stage 1 / Stage 2 / Stage 3 boundaries for telemetry, the new schema reports per-iteration boundaries instead — `iterations` is the number of triage cycles run.

## [2.9.0] - 2026-04-12

### Added
- **`--update` self-update command** — checks GitHub releases for newer versions and re-runs the platform-appropriate installer (shell script, PowerShell, or `cargo install`). Works from both `nd300 --update` and `speedqx --update`. Supports `--json` output.
- **Statistics module** (`src/speedtest/statistics.rs`) — full accuracy pipeline ported from the QubeTX web speed test:
  - **Modified trimean** (Ookla-style): `(P10 + 8*P50 + P90) / 10` — heavily median-weighted, robust against outliers
  - **30% slow-start discard** — eliminates TCP ramp-up contamination (up from 1-sample discard)
  - **IQR outlier filtering** with linear-interpolation percentiles
  - **Winsorized cross-validation** — caps extremes at 5th/95th percentiles and averages with IQR result if they diverge >15%
  - **Upload-specific pipeline** — keeps only fastest 50% of post-warmup samples before trimean (following Speedtest.net methodology)
  - **RFC 3550 jitter** — exponentially weighted moving average `J += (|D| - J) / 16` (replaces simple consecutive-diff)
  - **Bootstrap confidence intervals** — 1000-resample percentile method for download/upload uncertainty estimation
- **Inverse-variance weighted aggregation** — statistically optimal combination of provider bandwidth results, with weights clamped to [0.3, 0.7] to prevent degenerate cases
- **Confidence-weighted latency merge** — Cloudflare 40% / NDT7 60% (NDT7's kernel-level MinRTT is structurally superior)
- **Connection stability metrics** — coefficient of variation (CV) for download and upload, with stable/variable classification (threshold: CV < 15%)
- **Provider divergence detection** — flags when Cloudflare and NDT7 bandwidth results differ by more than 30%, indicating possible throttling or QoS policies

### Changed
- Slow-start discard increased from 1 sample to 30% of all samples across all providers
- Bandwidth aggregation changed from volume-weighted to inverse-variance weighted merge
- Jitter calculation changed from mean absolute consecutive difference to RFC 3550 exponentially weighted moving average
- IQR outlier filter now uses linear-interpolation percentiles instead of array-index quartiles
- All 4 providers now collect per-request Mbps samples for post-processing through the full statistics pipeline

## [2.8.0] - 2026-03-19

### Added
- **Quad-provider speed testing** — SpeedQX now runs 4 providers (Cloudflare, M-Lab NDT7, LibreSpeed, fast.com/Netflix) for maximum accuracy via volume-weighted aggregation
- **LibreSpeed provider** — new open-source HTTP-based speed test with automatic server selection from global server list and concurrent latency probing
- **fast.com (Netflix) provider** — speed test via Netflix OCA CDN with dynamic API token extraction for reliability; graceful degradation if Netflix changes their API
- **Duration guarantee** — `--duration` now means per-direction (30s = 30s download + 30s upload per provider); NDT7 uses wall-clock deadline loops instead of iteration-count approximation
- **Slow-start trimming** — all HTTP-based providers discard first transfer sample to exclude TCP slow-start from throughput calculations
- **Volume-weighted aggregation** — provider results weighted by data transferred (provider with more data contributes more to aggregate); minimum ping across providers
- **Outlier removal** — IQR-based outlier removal for throughput samples across iterations
- **Progress UX** — provider transition banners, per-provider summary after each completes, estimated test time at start, descriptive phase messages
- **Dynamic NDT7 upload frames** — upload frame size doubles from 8KB to 1MB during test for better saturation on fast connections

### Changed
- `--duration 30` now means 30 seconds per direction (download AND upload), not 30 seconds total split across both
- All providers now run mandatory in SpeedQX — removed `--cf-only` and `--ndt-only` flags
- fast.com defaults to `auto` duration; configurable via new `--fastcom-duration` flag
- Ping calculation changed from median to minimum across all providers (consistent with NDT7 MinRTT approach)
- nd300 diagnostic mode unchanged — still runs Cloudflare + NDT7 only for speed
- SpeedQX CLI description updated to "Quad-provider speed test"

### Fixed
- NDT7 duration not honored — 30s request previously produced ~40s+ due to iteration-count approximation; now uses wall-clock deadline loop

## [2.7.4] - 2026-03-12

### Fixed
- Fix NDT7 (M-Lab) WebSocket connection failing with 400 Bad Request — add required `Sec-WebSocket-Protocol: net.measurementlab.ndt.v7` subprotocol header to WebSocket handshake
- Add OS-native TLS certificate store support (`rustls-native-certs`) for WebSocket connections — resolves TLS validation failures in environments with custom/corporate CAs
- Add WSS→WS fallback for NDT7 WebSocket connections — if TLS connection fails, automatically retries with plain WebSocket URL from the M-Lab locate API

## [2.7.3] - 2026-03-12

### Added
- Man page generation (`nd300.1`, `speedqx.1`) via `clap_mangen` — auto-generated from CLI definitions at build time, included in release archives
- Usage examples section in SpeedQX `--help` output (matching nd300's existing examples)

### Changed
- Extracted CLI definitions to shared `src/cli.rs` module for reuse by both binaries and the man page generator

## [2.7.2] - 2026-03-12

### Changed
- Upgrade cargo-dist from 0.30.3 to 0.31.0 — fixes Node.js 20 deprecation warning in CI, updates GitHub Actions to current versions

## [2.7.1] - 2026-03-12

### Fixed
- Fix MSI installer CI failure — add `speedqx.exe` binary to `wix/main.wxs` manifest (was missing after v2.7.0 added the SpeedQX binary)

## [2.7.0] - 2026-03-12

### Added
- **SpeedQX** — new standalone speed test binary (`speedqx`) included in all release archives and installers alongside `nd300`
- **Dual-provider speed testing** — both `nd300` and `speedqx` now run Cloudflare and M-Lab NDT7 tests sequentially, averaging results for more accurate measurements
- **Latency metrics** — ping (median RTT), jitter (avg consecutive diff), and packet loss measured via Cloudflare HEAD probes; ping/jitter also extracted from NDT7 TCPInfo
- **NDT7 integration** — WebSocket-based download/upload tests against nearest M-Lab server with automatic server discovery
- **Per-provider breakdown** in technician mode (`nd300 -t`) — speed section now shows aggregated averages plus individual Cloudflare and NDT7 results with server info, data transferred, and durations
- **Ping in user mode** — speed summary line now shows ping: `242 Mbps down / 30 Mbps up (12ms)`
- **SpeedQX CLI flags** — `--cf-only`, `--ndt-only`, `--duration <secs|auto>`, `--latency-probes <N>`, `--json`, `--ascii`, `--no-color`
- `--uninstall` now removes both `nd300` and `speedqx` binaries

### Changed
- Speed test engine extracted to shared `speedtest` module used by both binaries
- **Breaking (JSON):** `speed_details` schema changed — now includes `ping_ms`, `jitter_ms`, `packet_loss_pct`, `providers[]` array with per-provider metrics, and `duration_s`
- Default nd300 speed test duration (10s) now runs both providers: CF gets 5s dl + 5s ul, NDT7 runs 1 iteration (~10s dl + ~10s ul)

## [2.6.0] - 2026-02-22

### Added
- Fix flow report generation — after `--fix` completes, a detailed Markdown report is saved to `~/Downloads/nd300-fix-report-YYYYMMDD-HHMMSS.md`
- Terminal fix summary printed after fix flow with result, likely root cause, changes applied, VPN activity, and report file path
- Root cause inference engine analyzes which steps resolved the issue (DNS misconfiguration, stale caches, degraded interface, corrupted stack)
- JSON mode `--fix --json` now includes `report_path` field pointing to the saved Markdown report
- Context-aware recommendations in the report based on what was fixed (DNS tips, driver suggestions, reboot advice)

### Changed
- Default DNS provider changed from Hybrid to Cloudflare (1.1.1.1) across `--dns`, `--fix` DNS fallback, and JSON mode — Hybrid (mixed providers) causes sticky failover issues
- DNS provider menu reordered: Cloudflare is now option 1 (recommended), Hybrid moved to last position with "not recommended" label
- Stage 3 macOS now sets Cloudflare DNS (1.1.1.1 + 1.0.0.1) instead of Hybrid (1.1.1.1 + 8.8.8.8) after network service recreation

## [2.5.0] - 2026-02-22

### Changed
- Fix flow Stage 1 now resets DNS to Automatic (DHCP) as its first step, removing any custom DNS servers before attempting other repairs. If connectivity is not restored, the DNS fallback still offers to set public DNS servers (Cloudflare, Google, etc.).

## [2.4.1] - 2026-02-15

### Changed
- Version short flag changed from `-V` to `-v` for convenience

## [2.4.0] - 2026-02-15

### Added
- `-d, --dns` flag for standalone DNS configuration — choose a provider, set DNS, verify connectivity, auto-revert on failure, then run full diagnostics
- NextDNS support (encrypted DNS with filtering) across all platforms:
  - Windows: DNS-over-HTTPS via `netsh` DoH template registration
  - Linux: DNS-over-TLS via `systemd-resolved` config, `nmcli` fallback
  - macOS: NextDNS CLI client integration with plain IP fallback
- Extended DNS provider menu: Hybrid, Cloudflare, Google, NextDNS, Automatic (DHCP)
- JSON output for `--dns` action with structured success/failure/revert fields
- Auto-revert to DHCP when DNS verification fails after change (includes platform-specific NextDNS cleanup)
- Man-page style `--help` output with grouped sections (Modes, Output, Speed Test, Actions) and usage examples

### Fixed
- Fix `extract_stat_value` dead code warning on Linux — function is only used in Windows `#[cfg]` block

## [2.3.1] - 2026-02-15

### Fixed
- Fix CI build failure on Linux targets — `TIMEOUT_QUICK` import was removed from `dns.rs` but is used inside `#[cfg(target_os = "linux")]` block
- Fix unused import warning for `TIMEOUT_SLOW` in `stages.rs` (used only on non-Windows platforms)
- Fix unnecessary `mut` warning on `renew_cmd` in `dhcp.rs` Linux DHCP renewal

## [2.3.0] - 2026-02-15

### Added
- DNS verification after connectivity checks in all fix flow stages — resolves 3 domains to confirm DNS is actually working, not just HTTP
- DNS server selection prompt when DNS fails: Hybrid (Cloudflare + Google, recommended), Cloudflare, Google, or Automatic (DHCP)
- DNS reachability test before committing server changes — detects corporate firewalls blocking public DNS and auto-adjusts
- Stage 3 macOS: set hybrid DNS (1.1.1.1 + 8.8.8.8) immediately after network service recreation, preventing DNS gap while DHCP delivers servers

### Fixed
- Fix `--fix` on macOS reporting "connected" when DNS is broken — HTTP captive portal check passed but DNS resolution failed after `configd` restart + DHCP reset race
- Fix Stage 2 DHCP renew failing on macOS Wi-Fi — added 5-second delay after interface re-enable for Wi-Fi association before DHCP request
- Fix Stage 3 macOS creating network service without DNS servers, leaving DNS broken until DHCP delivers servers

## [2.2.1] - 2026-02-13

### Fixed
- Fix `--fix` flow on macOS failing all Stage 1 steps when run without `sudo` — elevation check now runs before any stages, exiting immediately with a clear privilege hint instead of attempting commands that require root
- Fix `--fix` JSON mode silently running stages without elevation — now returns structured `elevated_privileges_required` error
- Fix macOS DNS flush reporting failure when `killall -HUP mDNSResponder` needs root but `dscacheutil -flushcache` succeeds — now reports partial success instead of error

## [2.2.0] - 2026-02-13

### Changed
- Replace WMI adapter enumeration with native Windows APIs (`ipconfig` crate + `GetIfEntry2`)
  - ~5ms vs ~300-500ms for adapter data collection
  - Accurate adapter type classification via `PhysicalMediumType` (Wi-Fi, Ethernet, Bluetooth, WWAN) instead of generic WMI "Ethernet 802.3"
  - Precise connection status: "No Cable" vs "Disabled" vs "Down" vs "Standby" via `AdminStatus` + `MediaConnectState`
- Replace per-adapter PowerShell `Get-NetAdapter` calls with `GetIfEntry2` for instant link speed/duplex in tech mode
- Replace WMI in `--fix` adapter listing with `ipconfig` crate for faster adapter detection
- Defer WMI driver query (`Win32_PnPSignedDriver`) to tech mode only — user mode no longer pays WMI cost
- Match drivers to adapters by hardware description instead of name substring (more reliable)

### Added
- Per-adapter link speed in user-mode summary (e.g., "Wi-Fi 866 Mbps" or "Wi-Fi 2.4 Gbps")
- Tech-mode NETWORK ADAPTERS section now shows: MAC address, TX/RX link speeds, gateway, DNS servers, MTU, IPv4 metric
- New JSON fields: `description`, `mac_address`, `link_speed_mbps`, `rx_link_speed_mbps`, `dns_servers`, `gateways`, `media_connect_state`, `physical_medium`, `mtu`, `ipv4_metric`
- `--fix` default interface detection now picks adapter with lowest IPv4 metric (route preference) instead of first active

## [2.1.0] - 2026-02-13

### Added
- Show adapter type names in diagnostic summary (e.g., "2 active (Ethernet, Wi-Fi)" instead of just "2 active")
- Normalize WMI adapter types for cleaner display (e.g., "Ethernet 802.3" -> "Ethernet")
- Show fix hint ("Run 'nd300 -f' to attempt automatic fixes") when failures are detected

### Changed
- Consolidate duplicate subprocess calls in technician mode via pre-fetched `SharedCache`
  - `netstat -ano` shared across connections, listening_ports, and connection_states (was 3 calls)
  - `ipconfig /all` shared across dhcp, vpn, and ipv6 (was 2 calls)
  - `sysinfo::Networks` shared across adapter_hw_stats and traffic_counters (was 2 calls)
  - `default_net::get_default_gateway()` shared with reverse_dns (was 2 calls)
  - Saves ~200-800ms in technician mode by eliminating redundant subprocess spawns
- Label "Bluetooth Device (Personal Area Network)" as "BT PAN" instead of "Bluetooth" in adapter summary to avoid confusion with Bluetooth radio status
- Improve virtual adapter detection with additional patterns: "host-only", "vm network"

## [2.0.3] - 2026-02-12

### Fixed
- Add retry logic to post-fix connectivity checks: retry up to 3 times with 30-second countdown between each attempt, preventing false failure reports after interface resets (especially on Windows where Wi-Fi reconnection takes 15-30+ seconds)
- Replace hardcoded 2s/5s/8s single-check waits with visible spinner countdown so the tool doesn't appear frozen during reconnection

## [2.0.2] - 2026-02-11

### Fixed
- Fix Windows service restart failure: replace `net stop/start` with `sc` commands for PPL-protected services (dnscache, Dhcp), with PowerShell fallback and graceful degradation instead of hard failure
- Remove redundant dim "Checking for VPN connections..." text that leaked raw ANSI escape codes (SGR 2 unsupported on some Windows terminals); VPN spinner already covers this

## [2.0.1] - 2026-02-11

### Fixed
- Add command timeouts to all subprocess calls in `--fix` flow to prevent indefinite hangs
  - Quick (15s): DNS flush, ARP flush, process queries, Wi-Fi scan
  - Medium (30s): Service restart, VPN vendor CLI, nmcli/scutil
  - Slow (60s): DHCP release/renew, interface disable/enable, netsh stack resets
- Add loading spinners to previously silent operations in `--fix` flow
  - VPN detection, disable, re-enable, and post-re-enable connectivity check
  - Post-stage connectivity wait periods (Stages 1, 2, 3)
  - Interface disable/re-enable wait gap in Stage 2
  - Wi-Fi rescan sleeps on Linux Stage 3
- Add retry on Stage 2 interface re-enable failure to prevent leaving interface disabled
- Fix missing `TIMEOUT_MEDIUM` import that would cause compile failure on Linux

## [2.0.0] - 2026-02-09

### Changed
- **Breaking:** `--fix` now uses a 3-stage graduated recovery instead of flat step sequence
  - Stage 1: Service restart (DNS flush, ARP flush, service restart, DHCP renew) — automatic
  - Stage 2: Interface reset (disable/re-enable default adapter, targeted DHCP) — prompts user
  - Stage 3: Network stack/profile reset (Winsock reset, profile deletion, Wi-Fi reconnect) — prompts with strong warning
- Connectivity checks between stages; stops as soon as connectivity is restored
- Linux adapter restart now supported (was previously skipped)
- JSON mode runs Stages 1-2 automatically; skips Stage 3 (requires interaction)

### Added
- VPN conflict detection and resolution integrated into `--fix` flow
  - Detects connected VPNs before fix stages
  - Enterprise VPN detection (Cisco, GlobalProtect, Zscaler, etc.) — never touched automatically
  - Consumer VPN disable via vendor CLI (NordVPN, ExpressVPN, Mullvad, Tailscale, WireGuard)
  - Fallback adapter-level disable for unknown VPNs
  - Offers to re-enable VPNs after fix; auto-disables again if connectivity breaks
- Enhanced Linux DNS flush: detects and flushes systemd-resolved, dnsmasq, and nscd layers
- VPN diagnostics now include vendor detection, enterprise classification, and OS interface name
- Structured JSON output with stage results, VPN section, and `requires_interaction` flag
- Wi-Fi SSID capture before disconnect for reconnection in Stage 3
- Cross-platform Wi-Fi network scanning for Stage 3 reconnection
- macOS Stage 3: network service removal/recreation with Keychain password retrieval
- Linux Stage 3: NetworkManager profile delete/recreate, wpa_supplicant fallback for dhcpcd systems

## [1.0.1] - 2026-02-08

### Fixed
- Fix non-exhaustive match in macOS DNS flush (compile error on macOS CI)
- Fix unused variable warnings on macOS/Linux targets

## [1.0.0] - 2026-02-08

### Fixed
- Replaced `.unwrap()` with safe fallback on JSON serialization in action modules (clear_dns, fix, uninstall)
- Preserve original Windows registry PATH type (REG_SZ vs REG_EXPAND_SZ) during uninstall

### Added
- `-c, --clear-dns` flag to flush DNS cache (Windows, macOS, Linux)
- `-f, --fix` flag for full network reset: DNS flush + ARP cache flush + DHCP lease renewal
- Interactive prompt to restart network adapters during `--fix` (Windows, macOS)
- `--uninstall` flag to completely remove nd300 from the system (binary, install receipt, PATH entry)
- JSON output support for all action flags

## [0.1.2] - 2026-02-08

### Fixed
- Fixed `&str` comparison in IPv6 diagnostic on Unix platforms

## [0.1.1] - 2026-02-08

### Fixed
- Added WiX GUIDs and MSI definition for cargo-dist MSI installer

## [0.1.0] - 2026-02-08

### Added
- Cross-platform network diagnostic tool (Windows, macOS, Linux)
- Two operating modes: User (clean summary) and Technician (deep diagnostics)
- 8 core diagnostic modules: adapters, interfaces, gateway, DNS, public IP, latency, speed test, port connectivity
- 17 technician-mode deep diagnostic modules: ARP, routing, connections, listening ports, DHCP, protocol stats, adapter HW, proxy, VPN, firewall, DNS cache, IPv6, MTU, connection states, bufferbloat, reverse DNS, TLS inspection, traffic counters
- Unicode box-drawing table rendering with ASCII fallback
- Color-coded status indicators (OK/Warn/Fail/Skip)
- JSON output mode for scripting
- Speed test against Cloudflare (configurable duration)
- Bufferbloat detection with grade scoring
- cargo-dist integration for automated cross-platform releases
- Shell installer (macOS/Linux) and PowerShell installer (Windows)
- MSI installer for Windows

### Fixed
- 19 audit fixes applied across all diagnostic modules
