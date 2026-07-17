# ND-300 Project Context

_Updated 2026-07-17._

## TL;DR

ND-300 is a cross-platform Rust CLI for network diagnostics, repair, and the
SpeedQX multi-provider speed test. The repository is preparing **v3.6.0**, a
combined macOS accuracy/safety and release-trust hardening release. The old
macOS deep reset that deleted/recreated a network service has been removed. Its
service-cycle replacement is deliberately unavailable until it passes an
offline-supervised disposable-Mac acceptance test. One user-authorized run of
the exact test path passed on this Mac under an independent root watchdog, but
that supplementary happy-path evidence is not a reason to enable production.
Never repeat it on a host whose Codex/SSH control depends on the selected link.

## Current Status

- Package/crates.io name: `nd300`; library: `nd_300`.
- Binaries: `nd300` and `speedqx`.
- Manifest version: `3.6.0` release candidate; last published baseline was
  `3.5.2`. Do not describe v3.6.0 as released until hosted evidence exists.
- Working branch: `codex/nd300-macos-hardening-v3.6.0`; repository default:
  `main`.
- Commands, flags, action IDs, exit codes, and existing serialized field shapes
  remain compatible.
- Windows's four first-class installer/update paths remain unchanged.
- Native/Rosetta execution, quarantined-artifact trust, Apple-accepted
  notarization for both architectures, exact-SHA hosted CI, crates.io, 28 assets
  plus attestations, update-from-v3.5.2, and Alienware smoke are pending release
  acceptance gates until independently evidenced.
- `.tasks/TASKS.md` is the authoritative Windows-readable handoff board. It records
  each remaining gate with its rationale, impact, commands, and acceptance
  criteria.

## v3.6.0 Scope

### Diagnostics

- The interface collector reconciles default route, current OS flags,
  hardware-port/network-service mapping, and addresses in one coherent internal
  snapshot. Only usable physical uplinks count as active; virtual/loopback/
  AWDL/LLW/dormant/link-local-only interfaces remain visible as detail. A
  default-route VPN and its physical carrier are reported separately. Sharing
  one immutable snapshot across the independently collected interface, VPN,
  route, DHCP, and Wi-Fi sections is recorded as follow-up in `.tasks/TASKS.md`.
- macOS core mode never waits for optional Wi-Fi metadata. Technician mode uses
  bounded `system_profiler` JSON with an older-text fallback, reads only the
  connected network, handles redacted SSIDs, and uses the reported band.
- Fixture-backed parsers cover macOS route/listener/IPv6-connection/DHCP/
  protocol-statistics/VPN/Wi-Fi formats. Suppressed counters are omitted rather
  than fabricated as zero.
- ULA IPv6 is distinct and `dual_stack` requires real IPv4 and IPv6
  connectivity. VPN detection correlates configuration, network state,
  addresses, and routes instead of treating every `utun` as active.
- Diagnostic subprocess timeouts kill/reap children. Apple networkQuality has
  progress heartbeats, a whole-phase deadline, and capped jittered backoff.
- TLS issuer/evidence comes from a bounded Rust handshake and is explicitly
  clear/detected/inconclusive. DNSSEC requires a valid signed control and a
  failed deliberately invalid control.

### Repair safety

- There is no macOS network-service delete/recreate path, Wi-Fi password
  retrieval, SSID scan, or incomplete service snapshot.
- The gated high-risk replacement maps one enabled physical service, snapshots
  exact DNS servers/search domains, registers `ReEnableMacosService` before
  disable, cycles off/on, and verifies enabled state plus expected route/
  reachability before resolving rollback.
- The live acceptance entrypoint is ignored/test-only and requires root, exact
  interface/service acknowledgements, and an independent offline watchdog. The
  production validation gate remains false without this evidence.
- DHCP renew runs only for an already-DHCP service; static/Manual/BootP/
  disabled/virtual/ambiguous services are not changed.
- Restoration drains LIFO after Ctrl-C, SIGTERM, SIGHUP, panic, timeout, or
  ordinary completion. Consumer VPN inverses register before disconnect;
  enterprise VPNs remain untouched. `reverted: true` requires verification.
- Elevated Unix operations validate the original account through numeric
  `SUDO_UID`/passwd data, drop privileges for user-scoped work, and atomically
  create invoking-user-owned mode-`0600` reports under that user's Downloads.

### Update, uninstall, and release trust

- Unix update keeps bounded Cargo first and never runs unsolicited
  `rustup update`. Both explicit invoking-user Cargo-bin binaries must report
  the exact version.
- The Unix fallback downloads the exact-tag target archive plus required
  sidecar under time/size limits, verifies SHA-256 in Rust, allowlist-extracts
  exactly two regular files, verifies both staged versions, checks pinned Apple
  trust on macOS, and replaces both transactionally with paired rollback.
- Unix uninstall follows proven origin: Cargo uses `cargo uninstall nd300`, an
  ordinary symlink removes only itself, a cargo-dist layout requires its valid
  receipt, and package-manager/local-build/unknown locations are refused.
- Mac release automation signs both binaries in arm64 and Intel archives with
  the pinned Developer ID fingerprint, Team ID, per-binary identifiers,
  hardened runtime, and timestamp; requires notarization status `Accepted`;
  repacks exact signed bytes; and regenerates checksums/manifest with no unsigned
  fallback. Host attests post-signing artifacts; later Windows assets get their
  own attestations.
- arm64 builds use `macos-15`, Intel uses `macos-15-intel`; deployment floors
  stay macOS 11.0 arm64 and 10.12 Intel. Release archives include `man/`.

## Architecture Map

- `src/actions/fix/`: bounded repair planner/runner, restoration, platform
  primitives, reports, DHCP/DNS/VPN/Wi-Fi safety.
- `src/actions/update.rs`: orchestration and unchanged Windows dispatch.
- `src/actions/unix_install.rs`: Unix origin detection, archive verification,
  staged validation, transaction/rollback, Cargo and uninstall primitives.
- `src/platform/invoking_user.rs`: validated Unix original-user context.
- `src/diagnostics/`: 8 core and 25 technician diagnostics plus fixtures.
- `src/speedtest/`: shared eight-source SpeedQX methodology implementation.
- `scripts/sign-notarize-macos.sh`: fail-closed Developer ID/notary/repack.
- `scripts/validate-macos-release.sh`: archive/floor/version/trust validation.
- `scripts/*macos-service-cycle*`: independent-watchdog destructive acceptance
  harness; never part of routine tests.
- `.github/workflows/ci.yml`: exact-SHA cross-platform quality/security gate.
- `.github/workflows/release.yml`: tag build, signing/notary, attestation/host.
- `.github/workflows/windows-installers.yml`: later Windows assets/attestation.
- `docs/architecture-decisions.md`: durable safety/release rationale.
- `.tasks/TASKS.md`: Windows-ready release/follow-up board with rationale and impact.

## Required Validation

```text
cargo fmt --all -- --check
cargo clippy --locked --workspace --all-targets -- -D warnings
cargo test --locked --workspace --all-targets
cargo build --release --locked
cargo package --locked --list
cargo publish --dry-run --locked
cargo audit
dist plan
actionlint .github/workflows/*.yml
shellcheck scripts/*.sh
```

Also inspect archives/man pages, run updater fault tests, read-only native macOS
core/technician JSON and table smokes, and native arm64/Rosetta/quarantine checks.
The live service cycle is a separate destructive acceptance procedure and must
use the independent watchdog/disposable environment described above.

## File Tree

Generated from the current workspace (excluding `.git`, build output, and
`.DS_Store`):

```text
.
├── .cargo
│   └── audit.toml
├── .claude
│   ├── README.md
│   ├── agents
│   │   ├── cross-platform-cfg-reviewer.md
│   │   └── installer-lockstep-reviewer.md
│   ├── hooks
│   │   ├── changelog_lockstep_reminder.py
│   │   └── wix_comment_guard.py
│   ├── settings.json
│   └── skills
│       ├── release
│       │   └── SKILL.md
│       └── validate-installers
│           ├── SKILL.md
│           └── scripts
│               └── validate-installers.ps1
├── .github
│   └── workflows
│       ├── ci.yml
│       ├── claude-code-review.yml
│       ├── claude.yml
│       ├── crates-publish.yml
│       ├── release.yml
│       └── windows-installers.yml
├── .gitignore
├── .mcp.json
├── AGENTS.md
├── CHANGELOG.md
├── CLAUDE.md
├── CODEX_PROJECT.md
├── Cargo.lock
├── Cargo.toml
├── HUMAN_CHANGELOG.md
├── LICENSE
├── METHODOLOGY.md
├── ND300-Project-Plan.md
├── README.md
├── .tasks
│   ├── TASKS.md
│   ├── MILESTONES.md
│   └── tasks
├── build.rs
├── docs
│   └── architecture-decisions.md
├── golden-vectors.json
├── inno
│   ├── corporate.iss
│   └── global.iss
├── man
│   ├── nd300.1
│   └── speedqx.1
├── scripts
│   ├── macos-service-cycle-live-runner.sh
│   ├── macos-service-cycle-watchdog.sh
│   ├── run-macos-service-cycle-live.sh
│   ├── test-macos-service-cycle-parsers.sh
│   ├── sign-notarize-macos.sh
│   └── validate-macos-release.sh
├── src
│   ├── actions
│   │   ├── clear_dns.rs
│   │   ├── dns.rs
│   │   ├── fix
│   │   │   ├── action.rs
│   │   │   ├── adapters.rs
│   │   │   ├── arp.rs
│   │   │   ├── cmd.rs
│   │   │   ├── connectivity.rs
│   │   │   ├── dhcp.rs
│   │   │   ├── dns.rs
│   │   │   ├── loop_runner.rs
│   │   │   ├── mod.rs
│   │   │   ├── report.rs
│   │   │   ├── session.rs
│   │   │   ├── stages.rs
│   │   │   ├── triage.rs
│   │   │   ├── vpn.rs
│   │   │   └── wifi.rs
│   │   ├── migrate.rs
│   │   ├── mod.rs
│   │   ├── uninstall.rs
│   │   ├── unix_install.rs
│   │   └── update.rs
│   ├── bin
│   │   └── speedqx.rs
│   ├── cli.rs
│   ├── config.rs
│   ├── diagnostics
│   │   ├── adapter_hw_stats.rs
│   │   ├── adapters.rs
│   │   ├── arp.rs
│   │   ├── bufferbloat.rs
│   │   ├── captive_portal.rs
│   │   ├── connection_states.rs
│   │   ├── connections.rs
│   │   ├── dhcp.rs
│   │   ├── dns.rs
│   │   ├── dns_benchmark.rs
│   │   ├── dns_cache.rs
│   │   ├── firewall.rs
│   │   ├── gateway.rs
│   │   ├── interfaces.rs
│   │   ├── ipv6.rs
│   │   ├── latency.rs
│   │   ├── listening_ports.rs
│   │   ├── mod.rs
│   │   ├── mtu.rs
│   │   ├── nat.rs
│   │   ├── ntp.rs
│   │   ├── packet_loss.rs
│   │   ├── ping.rs
│   │   ├── ports.rs
│   │   ├── protocol_stats.rs
│   │   ├── proxy.rs
│   │   ├── public_ip.rs
│   │   ├── reverse_dns.rs
│   │   ├── route_path.rs
│   │   ├── routing_table.rs
│   │   ├── shared_cache.rs
│   │   ├── speed.rs
│   │   ├── tls_inspection.rs
│   │   ├── traffic_counters.rs
│   │   ├── util.rs
│   │   ├── vpn.rs
│   │   └── wifi.rs
│   ├── error.rs
│   ├── lib.rs
│   ├── main.rs
│   ├── platform
│   │   ├── invoking_user.rs
│   │   └── mod.rs
│   ├── render
│   │   ├── bar.rs
│   │   ├── color.rs
│   │   ├── json.rs
│   │   ├── mod.rs
│   │   ├── progress.rs
│   │   ├── table.rs
│   │   ├── tech_mode.rs
│   │   └── user_mode.rs
│   └── speedtest
│       ├── adaptive.rs
│       ├── applenq.rs
│       ├── cachefly.rs
│       ├── cloudflare.rs
│       ├── display.rs
│       ├── fastcom.rs
│       ├── golden_tests.rs
│       ├── librespeed.rs
│       ├── mod.rs
│       ├── msak.rs
│       ├── ndt7.rs
│       ├── stat_primitives.rs
│       ├── statistics.rs
│       └── vultr.rs
├── wix
│   └── main.wxs
└── wix-corporate
    └── corporate.wxs
```
