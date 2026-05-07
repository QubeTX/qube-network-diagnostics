# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

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
