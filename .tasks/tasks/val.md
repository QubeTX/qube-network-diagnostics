TT;DR: Prove the repaired candidate locally on the Alienware, across all four built installers, and in disposable Windows jobs before merge. No network-repair mutation is allowed on the physical host.

## Why
The original Mac work was extensive but could not exercise Windows cfg branches, real Alienware adapter topology, WiX/Inno builds, or Windows installer ownership. This gate catches platform-specific failures before an immutable tag.

## Plan
Install Rust 1.97.0; run format, all-target/all-feature Clippy and tests, release build, audit, package list, publish dry-run, actionlint, ShellCheck, parser/fault scripts, and the validate-installers driver. Run the read-only core/technician/ASCII/version smoke and inspect against Get-NetAdapter/Get-NetRoute. In fresh GitHub Windows VMs, install 3.5.2 for four installer origins plus Cargo, upgrade directly to candidate outputs, verify fresh-shell ownership, dry-run migration, and uninstall behavior.

## Impact
Intended: release evidence covers physical Windows behavior and installer construction/lifecycle. Risks are confined to build artifacts and disposable VMs; physical-host commands must not invoke `fix`, service cycling, uninstall, or any network mutation before public release.

## Acceptance
Every local gate passes, all four installer builds exit zero, disposable upgrade cases pass, both candidate binaries report 3.6.0, the Intel BE202/default route is represented accurately, dormant adapters are not counted active, reports contain no credentials, and all smoke commands terminate within documented caps.

## Verification
- [x] Rust 1.97.0 format, Clippy, tests, release build, audit, package, and publish-dry-run pass
- [x] actionlint, ShellCheck, Mac parser/fault scripts, and cargo-dist plan pass
- [x] Global MSI, Corporate MSI, Global EXE, and Corporate EXE build with exit 0
- [ ] Five-origin disposable candidate-upgrade matrix passes
- [x] Alienware core JSON, technician JSON, ASCII, and version smokes pass without mutation or sensitive output

## Status
Local Alienware and installer validation is complete. Waiting only for the five-origin disposable candidate-upgrade matrix on the exact pushed PR head.

## Activity
- 2026-07-17 10:18 — imported ND36-002 and pre-tag portions of ND36-003 (agent: codex)
- 2026-07-17 11:31 — installed and selected Rust 1.97.0; format, all-target/all-feature Clippy with `-D warnings`, 310 workspace tests, release build, audit, package list, and publish dry-run all passed (agent: codex)
- 2026-07-17 11:35 — actionlint, ShellCheck 0.11, cargo-dist plan, PowerShell parsing, and macOS parser fixtures passed; package inventory contains 102 intended files and excludes `.tasks`/`.claude` (agent: codex)
- 2026-07-17 11:43 — validate-installers built Global MSI, Corporate MSI, Global EXE, and Corporate EXE with exit 0 using the locally provisioned WiX/Inno toolchains (agent: codex)
- 2026-07-17 11:51 — non-mutating Alienware smoke passed: both binaries report 3.6.0; core/technician JSON and ASCII completed successfully; the Intel BE202/default route was correctly represented as one active adapter; disconnected/virtual adapters did not inflate the summary; credential-pattern scans were clean (agent: codex)
- 2026-07-17 12:38 — exact head `89f3202` passed musl, ShellCheck, audit, cargo-dist, and the completed host lanes but failed Linux ARM before source compilation because the cross sysroot lacked libc headers; targeted CI toolchain fix prepared for a new exact head (agent: codex)
- 2026-07-17 12:51 — exact head `cffd19f` passed all ordinary CI including Linux ARM; the first disposable jobs exposed GUI-process waiting and strict-mode registry enumeration defects in the harness before lifecycle evidence collection, so no product lifecycle verdict was inferred from those five failures (agent: codex)
