# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

ND-300 (`nd300`) is a cross-platform CLI network diagnostics tool written in Rust. It runs 25+ diagnostic modules concurrently, supports user and technician modes, and includes a multi-stage network recovery system (`--fix`). Binary name is `nd300`.

## Build & Run Commands

```bash
cargo build --release          # Release build → target/release/nd300
cargo build                    # Debug build
cargo run -- [OPTIONS]         # Run with arguments
cargo run -- -t                # Technician mode (deep diagnostics)
cargo run -- --json            # JSON output
cargo run -- -f                # Run fix flow (requires elevation)
cargo run -- --fast            # Skip speed test
cargo install nd-300           # Install from crates.io
```

There are no tests or linting commands configured in this project. The CI/CD pipeline (`.github/workflows/release.yml`) only handles cross-platform release builds via `cargo-dist`.

## Architecture

### Core Flow

```
main.rs (clap CLI parser)
  → Exit-early actions: --fix, --clear-dns, --uninstall
  → config.rs (Config builder: colors, unicode, mode, format)
  → diagnostics/mod.rs (concurrent runner via tokio::join!)
    → 7 core diagnostics (concurrent)
    → Speed test (sequential — bandwidth-saturating)
    → 17 tech-mode diagnostics (concurrent, behind -t flag)
  → render/ (user_mode | tech_mode | json)
  → Exit code: 0=OK, 1=Warn, 2=Fail
```

### Module Layout

- **`src/actions/`** — Exit-early operations (`--fix`, `--clear-dns`, `--uninstall`). The fix flow (`actions/fix/`) implements 3-stage graduated network recovery with VPN detection, connectivity checks between stages, and Wi-Fi reconnection.
- **`src/diagnostics/`** — All diagnostic modules. Each module is self-contained with platform-specific code via `#[cfg]` attributes. `shared_cache.rs` pre-fetches subprocess outputs (netstat, ipconfig, sysinfo) to deduplicate calls across tech-mode modules.
- **`src/render/`** — Output formatting. `table.rs` builds Unicode/ASCII box-drawing tables. `color.rs` centralizes ANSI color output. `progress.rs` handles spinners and loading indicators.
- **`src/config.rs`** — Config builder with fluent API. Controls colors, unicode/ASCII, output format, mode selection, and box-drawing character sets.
- **`src/error.rs`** — Unified `AppError` enum via `thiserror`.
- **`src/platform/`** — OS detection and elevation checks.

### Key Patterns

**Platform abstraction:** Compile-time `#[cfg(target_os)]` branching per module — no runtime OS checks. Windows uses `winapi` (GetIfEntry2, TCP/UDP tables), `ipconfig` crate, and WMI (tech-mode only). Unix uses `nix` and `libc`.

**Subprocess timeouts:** All subprocess calls use explicit timeouts (`TIMEOUT_QUICK` 15s, `TIMEOUT_MEDIUM` 30s, `TIMEOUT_SLOW` 60s) defined in `actions/fix/cmd.rs`.

**SharedCache (tech mode):** `diagnostics/shared_cache.rs` pre-fetches netstat, ipconfig, sysinfo::Networks, and default gateway data once, shared across 10+ tech-mode diagnostic modules.

**DiagnosticStatus enum:** `Ok`, `Warn`, `Fail`, `Skip` — aggregated into exit code.

### Cross-Platform Build Targets

aarch64-apple-darwin, x86_64-apple-darwin, aarch64-unknown-linux-gnu, x86_64-unknown-linux-gnu, x86_64-unknown-linux-musl, x86_64-pc-windows-msvc

## Release Process

Releases are automated via `cargo-dist` v0.30.3. Push a version tag (e.g., `v2.2.0`) to trigger `.github/workflows/release.yml`, which builds for all 6 targets and generates shell/PowerShell/MSI installers.

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
| `winapi` 0.3 | Win32 API (Windows only) |
| `ipconfig` 0.3 | Windows adapter enumeration |
| `wmi` 0.14 | WMI queries (Windows tech-mode only) |
| `nix` 0.29 | Unix system calls |

## Fix Flow Stages

1. **Stage 1** (Automatic): DNS reset to DHCP, DNS flush, ARP flush, service restart, DHCP renew
2. **Stage 2** (Prompted): Interface disable/re-enable, targeted DHCP
3. **Stage 3** (Warned): Network stack reset, profile deletion, Wi-Fi reconnect

Connectivity is checked between stages; stops as soon as connectivity is restored. Enterprise VPNs (Cisco, Zscaler, GlobalProtect) are never auto-disabled.
