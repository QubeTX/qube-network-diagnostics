# ND-300 Network Diagnostic

[![Build Status](https://github.com/QubeTX/qube-network-diagnostics/actions/workflows/release.yml/badge.svg)](https://github.com/QubeTX/qube-network-diagnostics/actions/workflows/release.yml)
[![License: PolyForm Noncommercial](https://img.shields.io/badge/license-PolyForm%20Noncommercial-blue)](LICENSE)

Cross-platform network diagnostic tool for Windows, macOS, and Linux.

## Features

- **Two operating modes**: User mode (clean summary) and Technician mode (deep diagnostics)
- **8 core diagnostics**: adapters, interfaces, gateway, DNS, public IP, latency, speed test, port connectivity
- **17 deep diagnostics** (technician mode): ARP, routing, connections, listening ports, DHCP, protocol stats, adapter hardware, proxy, VPN, firewall, DNS cache, IPv6, MTU, connection states, bufferbloat, reverse DNS, TLS inspection, traffic counters
- **Speed test** against Cloudflare with configurable duration
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

```sh
# Default user mode — clean summary
nd300

# Technician mode — full deep diagnostics
nd300 -t

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

# Custom speed test duration (seconds)
nd300 --speed-duration 20

# Show help
nd300 --help
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
| `-c, --clear-dns` | Flush the system DNS cache |
| `-f, --fix` | Run the multi-stage network fix flow (requires elevated privileges) |
| `--uninstall` | Remove nd300 from the system (binary, install receipt, PATH entry) |
| `-h, --help` | Print help |
| `-V, --version` | Print version |

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
7. Speed Test (Cloudflare)
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
| **1** | Service Restart | DNS flush, ARP flush, service restart, DHCP renew. Automatic — no prompts. |
| **2** | Interface Reset | Disable/re-enable network interface, targeted DHCP renew. Prompted before starting. |
| **3** | Stack Reset | Full network service recreation (macOS), Winsock/TCP reset (Windows), profile reset (Linux). Strong warning before starting. |

Each stage verifies both HTTP connectivity and DNS resolution before declaring success. If DNS fails, the tool offers a choice of DNS servers:

- **Hybrid** (recommended) — Cloudflare primary (1.1.1.1) + Google secondary (8.8.8.8)
- **Cloudflare** — 1.1.1.1, 1.0.0.1 (privacy-focused)
- **Google** — 8.8.8.8, 8.8.4.4 (reliability)
- **Automatic** — DHCP-provided servers

The tool tests DNS server reachability before applying changes, automatically falling back if a server is blocked by corporate firewalls or ISP restrictions.

## Building from Source

```sh
git clone https://github.com/QubeTX/qube-network-diagnostics.git
cd qube-network-diagnostics
cargo build --release
```

The binary will be at `target/release/nd300` (or `nd300.exe` on Windows).

## License

[PolyForm Noncommercial 1.0.0](https://polyformproject.org/licenses/noncommercial/1.0.0/)
