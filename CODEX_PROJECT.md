# ND-300 Project Context

## TL;DR

ND-300 is a Rust CLI network diagnostics and recovery tool. It is published as the `nd300` crates.io package, and `main` pushes with a new version run the full release/deploy path automatically.

## Current Status

- Primary binaries: `nd300` and `speedqx`.
- Current package version: `3.0.5`.
- Crates.io package name: `nd300`.
- Active branch: `main`.
- Rust toolchain installed locally through `rustup` for this workspace run.
- `nd300 fix` uses a diagnostic-driven loop under `src/actions/fix/`.
- The current release workflow is seamless publishing: source verification, full cargo-dist release builds, GitHub release/updater assets, and crates.io publishing happen from the main-branch workflow when the manifest version is new.

## Project Goals

- Run cross-platform network diagnostics with clear user and technician output modes.
- Keep fix actions evidence-driven and bounded so healthy machines are not made worse.
- Preserve user network configuration whenever a repair primitive must mutate state.
- Share one accurate speed-test engine between `nd300` and `speedqx`.
- Continue supporting macOS, Windows, and Linux release targets through cargo-dist.
- Keep end-user installation and updates seamless through GitHub installer assets and `cargo install nd300`.

## Key Areas

- `src/actions/fix/triage.rs`: pure planner for deciding which fix actions are justified by current diagnostic failures.
- `src/actions/fix/loop_runner.rs`: iterative fix driver, confirmation handling, reporting, and re-probe flow.
- `src/actions/fix/stages.rs`: platform repair primitives, including macOS service recreation.
- `src/actions/fix/action.rs`: typed fix action registry and apply dispatch.
- `src/speedtest/`: shared Cloudflare, M-Lab NDT7, LibreSpeed, fast.com, and statistics aggregation code.
- `.github/workflows/release.yml`: main-branch and tag release workflow for source checks, cargo-dist builds, GitHub release assets, and crates.io publishing.

## Verification Plan

- `cargo fmt --check`
- `cargo test --lib`
- `cargo test`
- `cargo clippy --all-targets --all-features -- -D warnings`
- Non-mutating local fix verification where possible; elevated destructive checks should only run with explicit user approval.

## File Tree

```text
.
├── .github
│   └── workflows
│       ├── claude-code-review.yml
│       ├── claude.yml
│       └── release.yml
├── .gitignore
├── AGENTS.md
├── CHANGELOG.md
├── CLAUDE.md
├── CODEX_PROJECT.md
├── Cargo.lock
├── Cargo.toml
├── ND300-Project-Plan.md
├── README.md
├── build.rs
├── man
│   ├── nd300.1
│   └── speedqx.1
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
│   │   ├── mod.rs
│   │   ├── uninstall.rs
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
│   │   ├── connection_states.rs
│   │   ├── connections.rs
│   │   ├── dhcp.rs
│   │   ├── dns.rs
│   │   ├── dns_cache.rs
│   │   ├── firewall.rs
│   │   ├── gateway.rs
│   │   ├── interfaces.rs
│   │   ├── ipv6.rs
│   │   ├── latency.rs
│   │   ├── listening_ports.rs
│   │   ├── mod.rs
│   │   ├── mtu.rs
│   │   ├── ports.rs
│   │   ├── protocol_stats.rs
│   │   ├── proxy.rs
│   │   ├── public_ip.rs
│   │   ├── reverse_dns.rs
│   │   ├── routing_table.rs
│   │   ├── shared_cache.rs
│   │   ├── speed.rs
│   │   ├── tls_inspection.rs
│   │   ├── traffic_counters.rs
│   │   └── vpn.rs
│   ├── error.rs
│   ├── lib.rs
│   ├── main.rs
│   ├── platform
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
│       ├── cloudflare.rs
│       ├── display.rs
│       ├── fastcom.rs
│       ├── librespeed.rs
│       ├── mod.rs
│       ├── ndt7.rs
│       └── statistics.rs
└── wix
    └── main.wxs
```
