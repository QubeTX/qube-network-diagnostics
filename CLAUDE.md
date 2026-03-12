# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

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
cargo run -- -f                # Run fix flow (requires elevation)
cargo run -- --fast            # Skip speed test
```

There are no tests or linting commands configured. Validation happens via CI release builds across 6 platform targets.

## Architecture

### Core Flow

```
main.rs → Nd300Cli (clap parser, defined in src/cli.rs)
  → Exit-early actions: --fix, --clear-dns, --uninstall
  → Conditional exit: --dns (exits on failure, falls through to diagnostics on success)
  → config.rs (Config builder: colors, unicode, mode, format)
  → diagnostics/mod.rs (concurrent runner via tokio::join!)
    → 7 core diagnostics (concurrent)
    → Speed test (sequential — bandwidth-saturating)
    → 17 tech-mode diagnostics (concurrent, behind -t flag, uses SharedCache)
  → render/ (user_mode | tech_mode | json — selected by format + mode)
  → Exit code: 0=OK, 1=Warn, 2=Fail
```

### Module Layout

- **`src/cli.rs`** — Shared clap derive definitions (`Nd300Cli`, `SpeedQXCli`) used by both binaries and by `build.rs` for man page generation.
- **`src/actions/`** — Exit-early operations (`--fix`, `--clear-dns`, `--uninstall`, `--dns`). The fix flow (`actions/fix/`) implements 3-stage graduated network recovery with VPN detection, connectivity checks between stages, Wi-Fi reconnection, and report generation (`report.rs` saves Markdown to `~/Downloads/`).
- **`src/diagnostics/`** — All diagnostic modules. Each is self-contained with platform-specific code via `#[cfg]` attributes. Each returns `(DiagnosticResult, Option<DetailStruct>)` — the detail struct carries rich data for rendering. `shared_cache.rs` pre-fetches subprocess outputs to deduplicate calls across tech-mode modules.
- **`src/speedtest/`** — Dual-provider speed test engine (Cloudflare + M-Lab NDT7). `cloudflare.rs` and `ndt7.rs` are the provider clients; `display.rs` handles speedqx-specific progress rendering. Both binaries share this module.
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

Releases are automated via `cargo-dist` v0.31.0. Push a version tag (e.g., `v2.7.3`) to trigger `.github/workflows/release.yml`, which builds for all 6 targets and generates shell/PowerShell/MSI installers. The WiX manifest (`wix/main.wxs`) must include components for both `nd300.exe` and `speedqx.exe`.

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

## Fix Flow Stages

1. **Stage 1** (Automatic): DNS reset to DHCP, DNS flush, ARP flush, service restart, DHCP renew
2. **Stage 2** (Prompted): Interface disable/re-enable, targeted DHCP
3. **Stage 3** (Warned): Network stack reset, profile deletion, Wi-Fi reconnect

Connectivity is checked between stages; stops as soon as connectivity is restored. Enterprise VPNs (Cisco, Zscaler, GlobalProtect) are never auto-disabled. Fix reports are saved to `~/Downloads/nd300-fix-report-*.md`.
