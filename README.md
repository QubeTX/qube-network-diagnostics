# ND-300 Network Diagnostic

[![Build Status](https://github.com/QubeTX/qube-network-diagnostics/actions/workflows/release.yml/badge.svg)](https://github.com/QubeTX/qube-network-diagnostics/actions/workflows/release.yml)
[![License: PolyForm Noncommercial](https://img.shields.io/badge/license-PolyForm%20Noncommercial-blue)](LICENSE)

Cross-platform network diagnostic tool for Windows, macOS, and Linux. Includes **SpeedQX**, a standalone quad-provider speed test.

## Features

- **Two operating modes**: User mode (clean summary) and Technician mode (deep diagnostics)
- **8 core diagnostics**: adapters, interfaces, gateway, DNS, public IP, latency, speed test, port connectivity
- **17 deep diagnostics** (technician mode): ARP, routing, connections, listening ports, DHCP, protocol stats, adapter hardware, proxy, VPN, firewall, DNS cache, IPv6, MTU, connection states, bufferbloat, reverse DNS, TLS inspection, traffic counters
- **Diagnostic-driven `nd300 fix`** — runs the diagnostics, identifies which checks failed, and applies only the recovery actions that target those specific failures. Re-tests after each step and repeats until everything passes or no further actions remain. Works for technical and non-technical users; high-risk recovery steps always require Y/N confirmation with a plain-language explanation.
- **Quad-provider speed test** — Cloudflare + M-Lab NDT7 + LibreSpeed + fast.com (Netflix) with inverse-variance weighted aggregation, modified trimean (Ookla-style), and RFC 3550 jitter for technician-grade accuracy. Measures ping, jitter, download, upload, packet loss, stability, and provider divergence.
- **Self-update** — `nd300 update` / `speedqx update` checks GitHub for the latest release and reinstalls automatically
- **SpeedQX** standalone speed test binary — all 4 providers with per-provider breakdown and real-time progress
- **Bufferbloat detection** with grade scoring (A+ through F)
- **JSON output** for scripting and automation
- **Unicode box-drawing** table rendering with ASCII fallback
- **Color-coded** status indicators (OK/Warn/Fail/Skip)
- **Cross-platform** — native support for Windows, macOS, and Linux

## Installation

### Shell (macOS/Linux)

```sh
curl --proto '=https' --tlsv1.2 -LsSf https://github.com/QubeTX/qube-network-diagnostics/releases/latest/download/nd-300-installer.sh | sh
```

### PowerShell (Windows)

```powershell
powershell -ExecutionPolicy Bypass -c "irm https://github.com/QubeTX/qube-network-diagnostics/releases/latest/download/nd-300-installer.ps1 | iex"
```

### MSI (Windows)

Download the `.msi` installer from the [latest release](https://github.com/QubeTX/qube-network-diagnostics/releases/latest).

### Cargo

```sh
cargo install nd-300
```

### From Source

```sh
git clone https://github.com/QubeTX/qube-network-diagnostics.git
cd qube-network-diagnostics
cargo build --release
```

## Usage

### nd300 — Network Diagnostic

As of v3.0.0, every action command is available **either as a subcommand or as a flag** — both forms are first-class and produce identical behavior. Pick whichever you prefer.

```sh
# Default user mode — clean summary
nd300

# Technician mode — full deep diagnostics
nd300 -t

# Change DNS configuration (interactive)
nd300 -d

# Diagnostic-driven fix loop (subcommand form, recommended)
nd300 fix

# Same thing — legacy flag forms (still supported)
nd300 -f
nd300 --fix

# Auto-confirm Medium-cost prompts (does NOT bypass High-risk Y/N)
nd300 fix --yes
nd300 -f -y

# Skip the speed test for faster execution
nd300 --fast

# JSON output for scripting
nd300 --json
nd300 fix --json    # JSON works after subcommands too (global flag)

# ASCII characters instead of Unicode
nd300 --ascii

# Disable colored output
nd300 --no-color

# Custom report title
nd300 -T "Office Network Check"

# Custom speed test duration per provider (seconds)
nd300 --speed-duration 20

# Self-update — both forms work
nd300 update
nd300 --update

# Reset DNS cache — both forms work
nd300 clear-dns
nd300 --clear-dns
nd300 -c

# Uninstall — both forms work
nd300 uninstall
nd300 --uninstall

# Show help
nd300 --help
nd300 fix --help    # subcommand-specific help
```

### speedqx — Standalone Speed Test

```sh
# Full quad-provider speed test (Cloudflare + NDT7 + LibreSpeed + fast.com)
speedqx

# Custom duration per direction (30s download + 30s upload per provider)
speedqx --duration 60

# Override fast.com duration (defaults to "auto")
speedqx --fastcom-duration 30

# JSON output
speedqx --json

# Quick test with shorter duration
speedqx --duration 10 --latency-probes 5
```

## Example Output

```
╔══════════════════════════════════════════════════════════════╗
║                   ND-300 Network Diagnostic                 ║
╠══════════════════════════════════════════════════════════════╣
║ Category         │ Status │ Details                         ║
╠══════════════════╪════════╪═════════════════════════════════╣
║ Network Adapters │  OK    │ Wi-Fi (connected)               ║
║ IP Configuration │  OK    │ 192.168.1.42/24                 ║
║ Default Gateway  │  OK    │ 192.168.1.1                     ║
║ DNS Resolution   │  OK    │ 1.1.1.1, 8.8.8.8               ║
║ Public IP        │  OK    │ 203.0.113.45 (Cloudflare)       ║
║ Latency          │  OK    │ 12ms avg (1.1.1.1)             ║
║ Speed Test       │  OK    │ ↓ 245 Mbps  ↑ 48 Mbps          ║
║ Port Check       │  OK    │ 8/8 reachable                   ║
╚══════════════════╧════════╧═════════════════════════════════╝
```

## Command Line Options

| Flag | Description |
|------|-------------|
| `-t, --tech` | Technician mode — show full technical report with deep diagnostics |
| `-T, --title <TITLE>` | Custom title for the report header |
| `--json` | Output results as JSON |
| `--ascii` | Use ASCII characters instead of Unicode box-drawing |
| `--no-color` | Disable colored output |
| `--fast` | Skip the speed test (faster execution) |
| `--speed-duration <SECS>` | Speed test duration in seconds (default: 10, min: 4) |
| `--verbose` | Show additional debug/trace information |
| `-d, --dns` | Change DNS servers and verify connectivity (requires elevated privileges) |
| `-f, --fix` | Run the diagnostic-driven triage / fix loop (requires elevated privileges). Equivalent to `nd300 fix`. |
| `-c, --clear-dns` | Flush the system DNS cache. Equivalent to `nd300 clear-dns`. |
| `--uninstall` | Remove nd300 from the system. Equivalent to `nd300 uninstall`. |
| `--update` | Check for updates and install the latest version. Equivalent to `nd300 update`. |
| `-y, --yes` | Auto-confirm Medium-cost prompts when running the fix flow. Does **not** bypass High-risk Y/N. |
| `-h, --help` | Print help |
| `-v, --version` | Print version |

### Subcommands

| Subcommand | Equivalent flag | Description |
|------------|-----------------|-------------|
| `nd300 fix [-y]` | `nd300 -f` / `nd300 --fix` | Diagnostic-driven triage and recovery loop |
| `nd300 update` | `nd300 --update` | Self-update to the latest release |
| `nd300 clear-dns` | `nd300 -c` / `nd300 --clear-dns` | Flush the DNS cache and exit |
| `nd300 uninstall` | `nd300 --uninstall` | Uninstall nd300 from this system |

## SpeedQX Options

| Flag | Description |
|------|-------------|
| `--json` | Output results as JSON |
| `--ascii` | Use ASCII characters instead of Unicode box-drawing |
| `--no-color` | Disable colored output |
| `--duration <VALUE>` | Test duration per direction for CF/NDT7/LS: seconds or "auto" (default: 30) |
| `--fastcom-duration <VALUE>` | Test duration per direction for fast.com: seconds or "auto" (default: auto) |
| `--latency-probes <N>` | Number of latency probes (default: 20) |
| `--update` | Check for updates and install the latest version |
| `-v, --version` | Print version |
| `-h, --help` | Print help |

## User Mode vs Technician Mode

**User mode** (default) runs 8 core diagnostics and presents a clean summary table. Ideal for quick network health checks.

**Technician mode** (`-t`) runs all 8 core diagnostics plus 17 additional deep diagnostic modules. Produces a detailed technical report with per-module breakdowns, suitable for troubleshooting and support workflows.

## Diagnostics Covered

### Core Diagnostics (both modes)
1. Network Adapters
2. IP Configuration / Interfaces
3. Default Gateway
4. DNS Resolution
5. Public IP
6. Latency
7. Speed Test (Cloudflare + M-Lab NDT7 in nd300; all 4 providers in SpeedQX)
8. Port Connectivity

### Deep Diagnostics (technician mode only)
9. ARP Table
10. Routing Table
11. Active Connections
12. Listening Ports
13. DHCP Lease Info
14. Protocol Statistics
15. Adapter Hardware Details
16. Proxy Detection
17. VPN Detection
18. Firewall Status
19. DNS Cache
20. IPv6 Connectivity
21. MTU Discovery
22. Connection State Summary
23. Bufferbloat Detection
24. Reverse DNS
25. TLS Inspection

## Fix Flow (`nd300 fix` / `nd300 -f`)

The fix flow runs a **diagnostic-driven triage loop**: it tests the network, looks at what actually failed, applies only the recovery actions that target those specific failures, re-tests, and repeats. Requires elevated privileges (`sudo` on macOS/Linux, Administrator on Windows).

A clean network completes `nd300 fix` in under 8 seconds and applies zero actions. A real failure usually clears in 1–2 iterations.

### How it works

```text
1. Run baseline diagnostics
2. If everything passes → done.
3. Look at which specific diagnostics failed.
4. Group them by root cause (e.g. interface-down ⇒ skip DNS/gateway/IP — they cascade).
5. Pick the cheapest action that targets a remaining failure and apply it.
6. Wait for the system to stabilize.
7. Re-run the diagnostics.
8. Repeat until everything passes, or no further actions are available.
```

The loop is bounded by **three independent caps** so it always terminates:

- ≤ 6 iterations
- ≤ 4 minutes wall clock
- Per-action attempt cap (typically 1 — failed actions are not retried)

### Action ladder

Actions are tried in cost order: cheap first, expensive last. Each action declares which failure categories it can address; only relevant actions run for any given failure set.

| Cost / risk | Action | Targets |
|---|---|---|
| Cheap / Low | Flush DNS cache | DNS |
| Cheap / Low | Switch DNS to Cloudflare 1.1.1.1 | DNS |
| Cheap / Low | Reset DNS to router defaults | DNS |
| Cheap / Low | Flush ARP cache | Gateway, latency |
| Medium / Low | Restart networking services | DNS, gateway, public IP |
| Medium / Low | Renew DHCP lease | Gateway, public IP, adapters, interfaces |
| Medium / Medium | Temporarily disable consumer VPNs | Public IP, latency, DNS |
| Expensive / Medium | Restart the network adapter | Adapters, interfaces, gateway, DNS, public IP, latency |
| Expensive / **High** | Deep stack reset (Winsock / TCP-IP / IPv6 on Windows; recreate network service on macOS; recreate NetworkManager profile on Linux) | Last-resort recovery |

Enterprise VPNs (Cisco AnyConnect, Zscaler, Palo Alto / GlobalProtect, F5, Check Point, Juniper) are **never** auto-disabled.

### High-risk action prompts

The deep stack reset is gated behind a structured plain-language prompt:

```text
┌─ Escalating: Reset Windows networking stack ──────────────────────────────
Why I want to do this:
  Gateway and DNS are still failing after DHCP renew and ARP flush.
  Resetting the networking stack rebuilds Windows' TCP/IP, Winsock,
  and IPv6 catalogs from scratch — the standard fix when simpler
  steps haven't recovered the connection.

What will happen:
  • You will lose internet for ~10–15 seconds.
  • Open VPN sessions and SSH connections will drop.
  • A reboot is recommended afterward; nd300 will remind you at the end.

Reversible: requires reboot to fully revert
Typical duration: 10–15 seconds

Continue? Type 'y' to proceed, anything else to skip:
```

`--yes` does **not** bypass these prompts — they always require an explicit `y`. Anything else (including a blank Enter) is treated as N. In `--json` / non-interactive contexts, high-risk actions are skipped, not auto-applied, with a clear marker in the report.

### Hard-block detection

Some failure shapes can't be auto-fixed. The loop short-circuits cleanly with guidance instead of thrashing:

- **No physical link** — no cable / Wi-Fi.
- **ISP outage shape** — local network healthy, public internet failing.
- **Enterprise VPN active** — diagnostics shaped by a managed VPN we won't disable.

### Fix Report

Every run produces a Markdown report at `~/Downloads/nd300-fix-report-YYYYMMDD-HHMMSS.md`:

- **Verdict** — Fixed, Partially fixed, Couldn't fix, Cannot fix from here, Timed out, or Stopped at your request.
- **Baseline diagnostics** — full table snapshot before any action ran.
- **Iteration timeline** — for each loop pass: which actions ran, captured stdout/stderr/exit/duration, and the diagnostic delta after.
- **Final diagnostics** — same table as baseline so you can see what changed.
- **Environment** — OS, version, elevation status, VPNs detected.
- **What to try next** — concrete suggestions when the fix didn't fully succeed (restart router, contact ISP, check for driver updates, etc.).

In JSON mode (`nd300 fix --json`), the schema reports the same data as `outcome`, `iterations`, `applied_actions[]`, `remaining_failures[]`, `hard_block`, and `report_path`.

## DNS Configuration (`--dns`)

The DNS flag (`nd300 -d`) provides a standalone way to change your DNS configuration. Requires elevated privileges.

**Providers:**
- **Cloudflare** (recommended) — 1.1.1.1 (privacy-focused)
- **Google** — 8.8.8.8 (reliability)
- **NextDNS** — Encrypted DNS with filtering (requires config ID)
- **Automatic** — Reset to system default (DHCP)
- **Hybrid** (not recommended) — Cloudflare + Google

**NextDNS encryption by platform:**
- **Windows** — DNS-over-HTTPS via `netsh` DoH template registration
- **Linux** — DNS-over-TLS via `systemd-resolved` (falls back to plain IPs via `nmcli`)
- **macOS** — NextDNS CLI client (`nextdns install/activate`), plain IPs if CLI not installed

After setting DNS, the tool verifies both DNS resolution and HTTP connectivity. If verification fails, it automatically reverts to DHCP and reports the result. On success, full diagnostics run to confirm network health.

## Man Pages (Linux/macOS)

Man pages are generated at build time and included in release archives. To install after building from source:

```sh
sudo cp man/nd300.1 /usr/share/man/man1/
sudo cp man/speedqx.1 /usr/share/man/man1/
sudo mandb
```

Then use `man nd300` or `man speedqx` to view documentation.

## Building from Source

```sh
git clone https://github.com/QubeTX/qube-network-diagnostics.git
cd qube-network-diagnostics
cargo build --release
```

Binaries will be at `target/release/nd300` and `target/release/speedqx` (or `.exe` on Windows).

## Self-Update

Both binaries support self-updating to the latest release:

```sh
nd300 --update
speedqx --update
```

The update command:
1. Checks the latest release on GitHub
2. Compares against the current version
3. Downloads and runs the appropriate installer for your platform (shell script on macOS/Linux, PowerShell on Windows, or `cargo install` if installed via Cargo)

## Speed Test Methodology

The speed test uses a technician-grade accuracy pipeline matching the [SpeedQX web speed test](https://speedqx.com/how-it-works):

- **Modified trimean** (Ookla-style): `(P10 + 8*P50 + P90) / 10` for robust central tendency
- **30% slow-start discard**: eliminates TCP ramp-up contamination
- **IQR outlier filtering**: removes transient congestion and measurement artifacts
- **Winsorized cross-validation**: catches edge cases where IQR filtering is too aggressive
- **Upload-specific pipeline**: keeps fastest 50% of post-warmup samples (following Speedtest.net methodology)
- **RFC 3550 jitter**: exponentially weighted moving average, the standard used by VoIP and real-time media
- **Inverse-variance weighted aggregation**: statistically optimal combination of Cloudflare and NDT7 results
- **Provider divergence detection**: flags when results differ by >30%, indicating possible throttling or QoS

For a full technical breakdown, see the [SpeedQX Technical Report](https://speedqx.com/how-it-works).

## License

[PolyForm Noncommercial 1.0.0](https://polyformproject.org/licenses/noncommercial/1.0.0/)
