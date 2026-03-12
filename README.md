# ND-300 Network Diagnostic

[![Build Status](https://github.com/QubeTX/qube-network-diagnostics/actions/workflows/release.yml/badge.svg)](https://github.com/QubeTX/qube-network-diagnostics/actions/workflows/release.yml)
[![License: PolyForm Noncommercial](https://img.shields.io/badge/license-PolyForm%20Noncommercial-blue)](LICENSE)

Cross-platform network diagnostic tool for Windows, macOS, and Linux. Includes **SpeedQX**, a standalone dual-provider speed test.

## Features

- **Two operating modes**: User mode (clean summary) and Technician mode (deep diagnostics)
- **8 core diagnostics**: adapters, interfaces, gateway, DNS, public IP, latency, speed test, port connectivity
- **17 deep diagnostics** (technician mode): ARP, routing, connections, listening ports, DHCP, protocol stats, adapter hardware, proxy, VPN, firewall, DNS cache, IPv6, MTU, connection states, bufferbloat, reverse DNS, TLS inspection, traffic counters
- **Dual-provider speed test** — Cloudflare + M-Lab NDT7, averaged for accuracy. Measures ping, jitter, download, upload, and packet loss.
- **SpeedQX** standalone speed test binary — detailed results with per-provider breakdown
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

```sh
# Default user mode — clean summary
nd300

# Technician mode — full deep diagnostics
nd300 -t

# Change DNS configuration (interactive)
nd300 -d

# Multi-stage network fix
nd300 -f

# Skip the speed test for faster execution
nd300 --fast

# JSON output for scripting
nd300 --json

# ASCII characters instead of Unicode
nd300 --ascii

# Disable colored output
nd300 --no-color

# Custom report title
nd300 -T "Office Network Check"

# Custom speed test duration per provider (seconds)
nd300 --speed-duration 20

# Show help
nd300 --help
```

### speedqx — Standalone Speed Test

```sh
# Full dual-provider speed test (Cloudflare + NDT7)
speedqx

# Cloudflare only (skip NDT7)
speedqx --cf-only

# NDT7 only (skip Cloudflare)
speedqx --ndt-only

# Custom duration per provider (seconds or "auto")
speedqx --duration 60
speedqx --duration auto

# JSON output
speedqx --json

# Quick test with fewer latency probes
speedqx --latency-probes 5 --duration 10
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
| `-f, --fix` | Run the multi-stage network fix flow (requires elevated privileges) |
| `-c, --clear-dns` | Flush the system DNS cache |
| `--uninstall` | Remove nd300 from the system (binary, install receipt, PATH entry) |
| `-h, --help` | Print help |
| `-v, --version` | Print version |

## SpeedQX Options

| Flag | Description |
|------|-------------|
| `--json` | Output results as JSON |
| `--ascii` | Use ASCII characters instead of Unicode box-drawing |
| `--no-color` | Disable colored output |
| `--duration <VALUE>` | Test duration per provider: seconds or "auto" (default: 30) |
| `--cf-only` | Use only Cloudflare (skip NDT7) |
| `--ndt-only` | Use only M-Lab NDT7 (skip Cloudflare) |
| `--latency-probes <N>` | Number of latency probes (default: 20) |
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
7. Speed Test (Cloudflare + M-Lab NDT7)
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

## Fix Flow (`--fix`)

The fix flow (`nd300 -f`) runs a multi-stage network recovery process. Requires elevated privileges (`sudo` on macOS/Linux, Administrator on Windows).

| Stage | Action | Behavior |
|-------|--------|----------|
| **1** | Service Restart | Reset DNS to automatic (DHCP), DNS flush, ARP flush, service restart, DHCP renew. Automatic — no prompts. |
| **2** | Interface Reset | Disable/re-enable network interface, targeted DHCP renew. Prompted before starting. |
| **3** | Stack Reset | Full network service recreation (macOS), Winsock/TCP reset (Windows), profile reset (Linux). Strong warning before starting. |

Each stage verifies both HTTP connectivity and DNS resolution before declaring success. If DNS fails, the tool offers a choice of DNS servers:

- **Cloudflare** (recommended) — 1.1.1.1, 1.0.0.1 (privacy-focused)
- **Google** — 8.8.8.8, 8.8.4.4 (reliability)
- **Automatic** — DHCP-provided servers
- **Hybrid** (not recommended) — Cloudflare primary + Google secondary (mixed providers cause sticky failover)

The tool tests DNS server reachability before applying changes, automatically falling back if a server is blocked by corporate firewalls or ISP restrictions.

### Fix Report

After the fix flow completes, a detailed Markdown report is automatically saved to `~/Downloads/nd300-fix-report-YYYYMMDD-HHMMSS.md`. The report includes:

- **Summary** — resolved or not, at which stage, and the likely root cause
- **Per-stage step tables** — every action taken with OK/FAIL status and details
- **VPN activity** — any VPNs that were disabled during the fix
- **Recommendations** — context-aware next steps based on what was fixed

A concise terminal summary is also printed showing the result, likely cause, and report file path. In JSON mode (`--fix --json`), the `report_path` field points to the saved file.

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

## Building from Source

```sh
git clone https://github.com/QubeTX/qube-network-diagnostics.git
cd qube-network-diagnostics
cargo build --release
```

Binaries will be at `target/release/nd300` and `target/release/speedqx` (or `.exe` on Windows).

## License

[PolyForm Noncommercial 1.0.0](https://polyformproject.org/licenses/noncommercial/1.0.0/)
