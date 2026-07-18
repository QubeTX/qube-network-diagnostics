# ND-300 Network Diagnostic

[![Build Status](https://github.com/QubeTX/qube-network-diagnostics/actions/workflows/release.yml/badge.svg)](https://github.com/QubeTX/qube-network-diagnostics/actions/workflows/release.yml)
[![License: PolyForm Noncommercial](https://img.shields.io/badge/license-PolyForm%20Noncommercial-blue)](LICENSE)

Cross-platform network diagnostic tool for Windows, macOS, and Linux. Includes **SpeedQX**, a standalone eight-provider speed test (SpeedQX Methodology v4).

## Features

- **Two operating modes**: User mode (clean summary) and Technician mode (deep diagnostics)
- **8 core diagnostics**: adapters, interfaces, gateway, DNS, public IP, latency, speed test, port connectivity
- **25 deep diagnostics** (technician mode): ARP (+ gateway health), routing, connections, listening ports, DHCP, protocol stats, adapter hardware, proxy (+ PAC/WPAD), VPN, firewall, DNS cache, IPv6 (+ real v6 fetch), MTU (+ path-MTU probe), connection states, real bufferbloat (loaded latency during saturation), reverse DNS, TLS inspection, traffic counters, route path (traceroute analysis), sustained packet loss, NAT/CGNAT analysis, Wi-Fi link quality, DNS resolver benchmark (+ hijack/DNSSEC checks), captive portal detection, clock sync (NTP)
- **Diagnostic-driven `nd300 fix`** — runs the diagnostics, identifies which checks failed, and applies only the recovery actions that target those specific failures. Clean and latency-only networks are advisory/no-op, DNS repair is staged from safest to most invasive, and medium/high-risk steps require confirmation. Re-tests after each step and repeats until everything passes or no further actions remain.
- **Eight-source SpeedQX engine** — Cloudflare + M-Lab NDT7 + LibreSpeed + fast.com (Netflix) + M-Lab MSAK + Apple networkQuality + CacheFly + Vultr, using Methodology v4 capacity/consensus merging, agreement, confidence intervals, PDV jitter, and RPM. ND300's core diagnostic keeps the shorter Cloudflare + NDT7 run; standalone `speedqx` uses all eight.
- **Channel-preserving self-update** — `nd300 update` / `speedqx update` proves how the running copy was installed and updates through that same channel: Cargo stays Cargo, a managed macOS/Linux archive stays a managed archive, and each Windows MSI/EXE edition downloads its exact matching installer. Unknown, conflicted, local-build, and package-manager ownership is refused instead of guessed. Downloads are pinned to one exact release and verified before the two-binary transaction; a failure restores the old pair and points to the install page.
- **SpeedQX** standalone speed test binary — all 8 providers (Methodology v4: capacity + consensus merge, I² agreement, PDV jitter, RPM) with per-provider breakdown and real-time progress; `--fast` for the quick 3-provider early-stopping run, `--skip-msak`/`--skip-apple` to trim the full run
- **Bufferbloat detection** with grade scoring (A+ through F)
- **JSON output** for scripting and automation
- **Unicode box-drawing** table rendering with ASCII fallback
- **Color-coded** status indicators (OK/Warn/Fail/Skip)
- **Cross-platform** — native support for Windows, macOS, and Linux

## Installation

### Shell (macOS/Linux)

```sh
curl --proto '=https' --tlsv1.2 -LsSf https://github.com/QubeTX/qube-network-diagnostics/releases/latest/download/nd300-installer.sh | sh
```

### PowerShell (Windows)

```powershell
powershell -ExecutionPolicy Bypass -c "irm https://github.com/QubeTX/qube-network-diagnostics/releases/latest/download/nd300-installer.ps1 | iex"
```

### Windows installers (MSI + EXE, Global + Corporate)

Four first-class Windows installers are attached to every [release](https://github.com/QubeTX/qube-network-diagnostics/releases/latest). All four install **both** `nd300.exe` and `speedqx.exe` and add the install dir to your `PATH`. A fresh official installer represents your latest choice within the same Windows install scope: it removes a proven conflicting ND300 format before claiming ownership, or fails without claiming success when Windows permissions or scope make that unsafe.

| Edition | Scope | Admin (UAC)? | Installs to | Download |
|---|---|---|---|---|
| **Global** MSI | Per-machine | Yes | `C:\Program Files\nd300\bin\` | `nd300-x86_64-pc-windows-msvc.msi` |
| **Corporate** MSI | Per-user | No | `%LocalAppData%\Programs\nd300\bin\` | `nd300-x86_64-pc-windows-msvc-corporate.msi` |
| **Global** EXE | Per-machine | Yes | `C:\Program Files\nd300\bin\` | `nd300-x86_64-pc-windows-msvc-setup.exe` |
| **Corporate** EXE | Per-user | No | `%LocalAppData%\Programs\nd300\bin\` | `nd300-x86_64-pc-windows-msvc-corporate-setup.exe` |

- **Corporate** editions install per-user with **no admin prompt** — ideal for locked-down corporate machines where you can't elevate.
- Each installer writes a small `HKCU\Software\ND300\InstallSource` marker only after setup commits successfully, and removes it on uninstall only when it still names that installer. The marker helps `nd300 update` choose its matching in-place installer, but ND300 also proves ownership against the normalized Add/Remove Programs record; a Cargo-path binary or proven registered owner overrides a contradictory stale marker (see [Self-Update](#self-update)).
- Each installer has a `.sha256` sidecar; `nd300 update` verifies it before running the downloaded installer (refuse-on-mismatch).
- The cargo-dist PowerShell installer above remains available and installs into Cargo's bin directory.

**One install at a time — automatic, scope-safe consolidation.** A fresh MSI/EXE
proves and uninstalls a conflicting registered ND300 format in the same per-user
or per-machine scope without shell-expanding registry commands. Windows Installer
cannot safely major-upgrade across those two scopes, so an opposite-scope owner is
refused before any mutation; uninstall it first or choose the current installer
in the same scope. Each installer also offers two clean-up checkboxes, **both
checked by default**:

- **Remove an older Cargo-installed copy** — deletes a shadowing `nd300`/`speedqx`
  in your `~\.cargo\bin` (a prior `cargo install` copy usually wins on `PATH`
  otherwise). A non-running pair is removed before setup returns; a genuinely
  locked image uses the trusted retry helper. Your Rust toolchain is never
  touched — `cargo.exe`, `rustup.exe`, and the `.cargo\bin` `PATH` entry are left
  exactly as they were.
- **Also remove the other edition** — performs advisory, file-only cleanup of an
  unregistered stale Global/Corporate binary pair after ownership preflight.

The fallback clean-up only ever removes `nd300.exe`/`speedqx.exe`, never Cargo,
Rustup, the Cargo `PATH` entry, the running replacement, or anything in Downloads.
Supported registered takeover removes the prior binary pair, Add/Remove Programs
entry, its correct user/system `PATH` entry, and stale marker together. MSI product
codes are accepted only after Windows Installer itself proves that they belong to
ND300's canonical UpgradeCode. If scope or permissions make takeover unsafe, setup
stops with guidance rather than leaving two channels and pretending consolidation
succeeded. These helpers are installer-only; you never need to run them by hand.

### Cargo

```sh
cargo install nd300
```

Older installed copies that still request `nd-300-installer.sh` or `nd-300-installer.ps1` are supported by compatibility aliases on every new GitHub release, so the legacy updater path can still reach the current installers.

Cargo itself cannot run ND300-specific post-install hooks for unrelated old install locations. A Cargo-managed `nd300 update` stays on Cargo and safely retires its running executable, but a deliberate switch *to* bare `cargo install` should be preceded by `nd300 uninstall` if a Windows installer currently owns the command. The official MSI/EXE and PowerShell entrypoints can perform their own scope-safe fresh-channel takeover automatically.

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
nd300 dns          # subcommand form (recommended); on success continues to diagnostics
nd300 -d           # legacy flag form (still supported); identical behavior

# Diagnostic-driven fix loop (subcommand form, recommended)
nd300 fix

# Same thing — legacy flag forms (still supported)
nd300 -f
nd300 --fix

# Auto-confirm medium-risk prompts (does NOT bypass high-risk Y/N)
nd300 fix --yes
nd300 -f -y

# Skip the speed test for faster execution
nd300 --fast
nd300 fix --fast

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
# Full eight-provider speed test (Cloudflare + NDT7 + MSAK + LibreSpeed + fast.com + CacheFly + Vultr + Apple)
speedqx

# Custom duration per direction (60s download + 60s upload per provider)
speedqx --duration 60

# Override fast.com duration (defaults to "auto")
speedqx --fastcom-duration 30

# Classic 4-provider run
speedqx --skip-msak --skip-apple

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
| `--speed-duration <SECS>` | Speed test duration in seconds, per direction per provider (default: 15, min: 4) |
| `--verbose` | Show additional debug/trace information |
| `-d, --dns` | Change DNS servers and verify connectivity (requires elevated privileges) |
| `-f, --fix` | Run the diagnostic-driven triage / fix loop (requires elevated privileges). Equivalent to `nd300 fix`. |
| `-c, --clear-dns` | Flush the system DNS cache. Equivalent to `nd300 clear-dns`. |
| `--uninstall` | Remove nd300 from the system. Equivalent to `nd300 uninstall`. |
| `--update` | Check for updates and install the latest version. Equivalent to `nd300 update`. |
| `-y, --yes` | Auto-confirm medium-risk prompts when running the fix flow. Does **not** bypass high-risk Y/N. |
| `-h, --help` | Print help |
| `-v, --version` | Print version |

### Subcommands

| Subcommand | Equivalent flag | Description |
|------------|-----------------|-------------|
| `nd300 dns` | `nd300 -d` / `nd300 --dns` | Change DNS servers and verify connectivity (semi-exit-early: exits on failure, continues to diagnostics on success) |
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
| `--duration <VALUE>` | Test duration per direction for CF/NDT7/LS/MSAK/Apple: seconds or "auto" (default: 30) |
| `--fastcom-duration <VALUE>` | Test duration per direction for fast.com: seconds or "auto" (default: auto) |
| `--latency-probes <N>` | Number of latency probes (default: 20) |
| `--skip-msak` | Skip the M-Lab MSAK multi-stream provider |
| `--skip-apple` | Skip the Apple networkQuality provider |
| `--update` | Check for updates and install the latest version |
| `-v, --version` | Print version |
| `-h, --help` | Print help |

## User Mode vs Technician Mode

**User mode** (default) runs 8 core diagnostics and presents a clean summary table. Ideal for quick network health checks.

**Technician mode** (`-t`) runs all 8 core diagnostics plus 25 additional deep diagnostic modules. Produces a detailed technical report with per-module breakdowns, suitable for troubleshooting and support workflows. A full technician run takes ~2–4 minutes — the long probes (traceroute, sustained packet loss, loaded bufferbloat) are doing real work.

## Diagnostics Covered

### Core Diagnostics (both modes)
1. Network Adapters
2. IP Configuration / Interfaces
3. Default Gateway
4. DNS Resolution
5. Public IP
6. Latency
7. Speed Test (Cloudflare + M-Lab NDT7 in nd300; all 8 providers in SpeedQX)
8. Port Connectivity

### Deep Diagnostics (technician mode only)
9. ARP Table + Gateway Health
10. Routing Table
11. Active Connections
12. Listening Ports
13. DHCP Lease Info
14. Protocol Statistics
15. Adapter Hardware Details
16. Proxy Detection (+ PAC validation, WPAD discovery)
17. VPN Detection
18. Firewall Status
19. DNS Cache
20. IPv6 Connectivity (+ real HTTPS-over-v6 fetch, v4-vs-v6 comparison)
21. MTU per Interface + Path-MTU Discovery
22. Connection State Summary
23. Bufferbloat (real loaded-latency measurement, per-direction grades)
24. Reverse DNS
25. TLS Inspection
26. Traffic Counters
27. Route Path (traceroute with LAN/ISP/backbone analysis)
28. Sustained Packet Loss (30 probes × 3 targets)
29. NAT Analysis (double-NAT, CGNAT detection)
30. Wi-Fi Link Quality (signal, channel, band, PHY, security)
31. DNS Resolver Benchmark (+ NXDOMAIN-hijack and DNSSEC checks)
32. Captive Portal Detection
33. Clock Sync (SNTP offset measurement)

### macOS accuracy notes

ND300 builds one topology view from the default route, current interface flags,
network-service/hardware-port mappings, and assigned addresses. The active count
includes usable physical uplinks, not loopback, AWDL/LLW, dormant tunnels, or
link-local-only devices. If a VPN owns the default route, the logical tunnel and
underlying physical uplink are described separately.

Core mode never waits for optional Wi-Fi radio metadata. Technician mode uses a
bounded `system_profiler` query, with a text fallback for older supported macOS
versions; it reads only the connected network and treats a privacy-redacted SSID
as unavailable. VPN reporting correlates macOS VPN configuration, network state,
addresses, and routes rather than calling every `utun` device a VPN. IPv6 ULA is
reported separately, and `dual_stack` means both IPv4 and IPv6 connectivity were
actually verified.

TLS inspection reports **clear**, **detected**, or **inconclusive** evidence and
populates the issuer only after a bounded Rust TLS handshake. DNSSEC is reported
as validating only when a valid signed control resolves and the deliberately
invalid control does not. If macOS suppresses protocol counters, that section is
omitted instead of presenting unknown values as zero.

## Fix Flow (`nd300 fix` / `nd300 -f`)

The fix flow runs a **diagnostic-driven triage loop**: it tests the network, looks at what actually failed, applies only the recovery actions that target those specific failures, re-tests, and repeats. Requires elevated privileges (`sudo` on macOS/Linux, Administrator on Windows).

A clean network completes `nd300 fix` in under 8 seconds and applies zero actions. Latency-only findings are reported as advisory because ICMP can be rate-limited or deprioritized by healthy networks. A real failure usually clears in 1–2 iterations.

### How it works

```text
1. Run baseline diagnostics
2. If everything passes → done.
3. Look at which specific diagnostics failed.
4. Group them by root cause (e.g. interface-down ⇒ skip DNS/gateway/IP — they cascade).
5. Pick the safest justified action that targets a remaining failure and apply it.
6. Wait for the system to stabilize.
7. Re-run the diagnostics.
8. Repeat until everything passes, or no further actions are available.
```

The loop is bounded by **three independent caps** so it always terminates:

- ≤ 6 iterations
- ≤ 4 minutes wall clock
- Per-action attempt cap (typically 1 — failed actions are not retried)

### Action ladder

Actions are tried by evidence and risk: cheap first, reversible first, and only when the current failure set justifies the action. DNS repair is intentionally staged so the tool flushes caches and restores router-provided DNS before considering a public DNS change.

| Cost / risk | Action | Targets |
|---|---|---|
| Cheap / Low | Flush DNS cache | DNS |
| Cheap / Low | Reset DNS to router defaults | DNS |
| Cheap / Low | Switch DNS to Cloudflare 1.1.1.1 | DNS, only after safer DNS-specific fixes fail |
| Cheap / Low | Flush ARP cache | Gateway |
| Medium / Low | Restart networking services | DNS, gateway, public IP |
| Medium / Low | Renew DHCP lease | Gateway, public IP, adapters, interfaces |
| Medium / Medium | Temporarily disable consumer VPNs | Public IP, DNS; interactive confirmation required |
| Expensive / Medium | Restart the network adapter | Adapters, interfaces, gateway, DNS, public IP |
| Expensive / **High** | Deep stack reset (Winsock / TCP-IP / IPv6 on Windows; gated network-service cycle on macOS; recreate NetworkManager profile on Linux) | Last-resort recovery |

Enterprise VPNs (Cisco AnyConnect, Zscaler, Palo Alto / GlobalProtect, F5, Check Point, Juniper) are **never** auto-disabled. Consumer VPN disable is also skipped in JSON or non-interactive mode, even with `--yes`, because the tool cannot safely guide re-enable/recovery steps there. Other medium-risk actions such as DHCP reset, service restart, adapter restart, and public-DNS changes are skipped in JSON or non-interactive mode unless `--yes` can safely auto-confirm the medium-risk prompt. High-risk actions are never auto-confirmed.

### Confirmation prompts

Medium-risk actions explain what they will change and require confirmation unless `--yes` is provided. The deep stack reset is high-risk and is always gated behind a structured plain-language prompt:

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

`--yes` does **not** bypass high-risk prompts — they always require an explicit `y`. Anything else (including a blank Enter) is treated as N. In `--json` / non-interactive contexts, high-risk actions are skipped, not auto-applied, with a clear marker in the report.

#### macOS deep-reset safety

ND300 no longer deletes and recreates a macOS network service. That older design
could lose service identity/order and only partially restore Wi-Fi, DNS, search,
proxy, or addressing state when any command failed. The deletion path, Wi-Fi
password retrieval, and SSID scanning have been removed.

The replacement maps the exact enabled physical service and cycles it off/on. It
registers re-enable before mutation, preserves DNS servers and search domains
exactly, and requires the service to be enabled with the expected physical route
and reachability before the restore is considered complete. DHCP renewal likewise
runs only when that service is already configured for DHCP; Manual, BootP,
static, disabled, virtual, missing, or ambiguous services are not changed.

The macOS service cycle is **fail-closed and unavailable in this release build
until its destructive acceptance test passes under an independent,
offline-capable watchdog on a disposable Mac VM or sacrificial service**. A Mac
whose control session depends on the Wi-Fi being cycled is not a safe acceptance
host: losing Wi-Fi would also lose the recovery operator. Until the gate is
deliberately enabled after that proof, ND300 gives safe lower-risk/manual guidance
and does not cycle the service.

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

When the command is elevated through `sudo`, ND300 validates the invoking user
from numeric account data and writes the report to that user's Downloads folder,
not `/var/root`. Reports are created atomically with private `0600` permissions
and correct ownership. Restore operations run newest-first after Ctrl-C, SIGTERM,
or SIGHUP, and `reverted` is reported only after state verification.

In JSON mode (`nd300 fix --json`), the schema reports the same data as `outcome`, `iterations`, `applied_actions[]`, `remaining_failures[]`, `hard_block`, and `report_path`.

## DNS Configuration (`nd300 dns`)

`nd300 dns` (or the legacy flag `nd300 -d` / `--dns`) provides a standalone way to change your DNS configuration. Requires elevated privileges. The subcommand and flag are identical: both are semi-exit-early — they exit on failure, and on success fall through to running standard diagnostics so you can immediately confirm the change improved connectivity.

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

After setting DNS, the tool verifies both DNS resolution and HTTP connectivity.
On macOS it first captures the exact prior DNS servers **and search domains**;
if capture fails, no change is made, and if verification fails the exact snapshot
is restored and read back before `reverted: true` is reported. Windows/Linux
retain their automatic-DNS rollback for a failed provider change. On success,
full diagnostics run to confirm network health.

## Man Pages (Linux/macOS)

Man pages are generated at build time and included in release archives. To install after building from source:

macOS (user-writable, no `sudo` or `mandb`):

```sh
mkdir -p "$HOME/.local/share/man/man1"
cp man/nd300.1 man/speedqx.1 "$HOME/.local/share/man/man1/"
export MANPATH="$HOME/.local/share/man:${MANPATH:-}"
```

Add the `MANPATH` export to `~/.zprofile` if you want it in every new shell.

Linux system-wide:

```sh
sudo install -Dm644 man/nd300.1 /usr/local/share/man/man1/nd300.1
sudo install -Dm644 man/speedqx.1 /usr/local/share/man/man1/speedqx.1
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

Both binaries support self-updating to the latest release. Either form works:

```sh
nd300 update          # subcommand form (recommended)
nd300 --update        # legacy flag form (still supported)
speedqx update
speedqx --update
```

The updater follows a **prove, pin, verify, replace** contract:

1. Checks the latest release on GitHub. If you're already on it, exits 0 with no action.
2. Proves the running installation channel. Cargo registry metadata selects only
   bounded `cargo install nd300 --version =<resolved> --force --locked` as the
   validated invoking user. A valid Unix receipt selects only the native archive
   transaction. Windows registry,
   path, marker, and Cargo evidence select exactly one of Cargo, standalone, or
   MSI/EXE × Global/Corporate. A conflict or unknown owner changes nothing.
3. Public commands and download links use `releases/latest`; after the latest tag
   is resolved, every archive, installer, checksum, and verification step is
   pinned to that one immutable tag. No transaction mixes moving “latest” bytes.
4. A managed macOS/Linux archive fetches the exact compile-target `.tar.xz` plus
   required `.sha256` sidecar with time and size limits; verifies SHA-256 in
   ND300; and allowlist-extracts only the two expected regular files. Absolute
   paths, traversal, symlinks, hardlinks, duplicates, and unexpected layouts are
   rejected.
5. Executes both staged programs with `--version`. On macOS it additionally
   requires the pinned Developer ID certificate, Team ID `M9D5379H93`, identifiers
   `com.qubetx.nd300` and `com.qubetx.speedqx`, hardened runtime, and timestamp.
6. Atomically replaces both binaries with backups. Any failure restores both;
   old and new ND300/SpeedQX versions are never intentionally mixed.
7. On Windows, a Cargo-bin path wins over any stale installer marker. Otherwise
   ND300 proves a matching MSI/Inno owner from Add/Remove Programs and its
   normalized install location before selecting one of the four installer-aware
   update paths. MSI identity is accepted only when Windows Installer enumerates
   the product under ND300's canonical UpgradeCode; the marker is only a compatible
   fallback. The two running-image files are renamed to versioned, allowlisted
   siblings before
   replacement, restored on failure, and removed after the old process exits.
8. There is no cross-channel fallback after a failed update. The old installation
   remains authoritative, pretty and `--json` output explain the failure, and
   `https://reports.qubetx.com/nd300#install` is provided for a fresh download.

Unix uninstall is origin-aware too. Cargo-managed installs use
`cargo uninstall nd300`; invocation through an ordinary symlink removes only the
symlink; a cargo-dist layout requires its validated receipt. ND300 refuses to
modify unknown, local-build, Homebrew/MacPorts, or other package-manager-owned
locations and prints manual guidance instead.

### Windows installer-aware self-update

If you installed via one of the four first-class Windows installers (MSI/EXE × Global/Corporate — see [Installation](#windows-installers-msi--exe-global--corporate)), `nd300 update` matches the running executable to its normalized HKCU/HKLM Add/Remove Programs record and downloads the **matching** installer for an in-place upgrade — rather than switching you to a different installer/format. The `HKCU\Software\ND300\InstallSource` marker and known install path are fallbacks, not unconditional authority. A binary in `.cargo\bin`, or a single proven registered owner, overrides a contradictory stale marker.

Windows locks a running executable against overwrite but permits an in-place
rename. Before replacement, the updater/installer moves only `nd300.exe` and
`speedqx.exe` to versioned `*.update-old-*` siblings. MSI performs that move in
its rollback-aware transaction; Inno and the hardened cargo-dist script restore
the pair if their copy phase fails. The new allowlisted helper removes the
retired images immediately or after the old updater exits, so no reboot is
required merely to replace the running command.

Before running a downloaded MSI/EXE, the updater pins both the installer and its `.sha256` sidecar to the already-resolved release tag and verifies the download against it (refuse-on-mismatch) — defending against a corrupted download or a network MITM (corporate TLS-interception proxies, hostile WiFi). After the installer exits, it re-runs `--version` to confirm the file replacement actually took effect (and surfaces the reboot-required case honestly if Windows scheduled a deferred replace).

**Updates preserve channel; fresh installs may change format within one scope.** An MSI update stays the same MSI edition, an Inno update stays the same EXE edition, Cargo stays Cargo, and standalone stays standalone. A deliberate fresh official Windows install treats the selected package as the latest user intent and may replace a proven MSI/EXE owner in the same per-machine or per-user scope. A raw per-user ↔ per-machine switch is refused before mutation because Windows Installer cannot major-upgrade across contexts and an elevated installer must never execute a user-writable uninstaller. Uninstall the old scope first, or choose the desired format in that same scope. The Global MSI also repairs the pre-v3.1 `C:\Program Files\nd-300\bin` system-PATH entry after a successful migration while preserving every unrelated entry and registry value type. Default Cargo-copy cleanup remains narrowly file-only and never removes the Rust toolchain.

Windows uninstall is ownership-aware as well. For a proven MSI/Inno install,
`nd300 uninstall` starts that registered uninstaller directly from a validated
product code or uninstaller path—never a shell-expanded registry command—so the
binary pair, Add/Remove Programs registration, correct user/system PATH entry,
and installer marker are removed together. Cargo and portable copies keep the
strict two-binary allowlist and never remove Cargo, Rustup, or the Cargo PATH.

Versioning is prerelease-aware: a prerelease of an upcoming version is treated as newer than the previous stable patch, and a stable release is newer than its own prerelease. GitHub's unauthenticated rate-limit case (60 requests/hour per IP) is named explicitly so you know to just wait.

Current releases publish the canonical `nd300-installer.sh` and `nd300-installer.ps1` assets plus legacy `nd-300-installer.*` aliases for older installed copies.

### Update JSON output

In `--json` mode, the response includes `"strategy"` (the precise variant that ran or was attempted) and, on failure, an `"attempts"` array. The legacy `"method"` field still maps to `"cargo"` or `"installer"` for backward compatibility. On **Windows**, every update payload also carries a top-level `"install_origin"` field — one of `msi-global`, `msi-corporate`, `exe-global`, `exe-corporate`, `cargo-or-installer`, or `unknown` (the field is omitted on macOS/Linux, where install origin doesn't vary).

```json
{
  "action": "update",
  "success": true,
  "current_version": "3.5.2",
  "latest_version": "3.6.0",
  "update_available": true,
  "method": "installer",
  "strategy": "msi_corporate",
  "install_origin": "msi-corporate"
}
```

The precise `"strategy"` values are `cargo`, `unix_archive`,
`installer_powershell`, `installer_pwsh`, `msi_global`, `msi_corporate`,
`exe_global`, and `exe_corporate`.

## Speed Test Methodology

The CLI implements [SpeedQX Methodology v4](./METHODOLOGY.md), byte-identical
with the website/app specification and protected by shared golden vectors:

- Dense warmed latency sampling and adaptive throughput transfers.
- Plateau-based warm-up removal, IQR filtering, modified trimean, and a
  Hodges–Lehmann stability cross-check per provider.
- Circular block bootstrap with BCa 95% intervals for autocorrelated samples.
- A capability-aware **capacity** headline plus conservative all-provider
  **consensus**, random-effects heterogeneity, HKSJ confidence intervals, and a
  70% per-provider weight cap.
- I² agreement bands, PDV jitter, bufferbloat deltas, approximate RPM, and honest
  provider availability/exclusion fields.
- An anytime-valid empirical-Bernstein confidence sequence for `--fast`, avoiding
  optional-stopping bias.

For the full equations, thresholds, provider registry, and parity contract, see
[METHODOLOGY.md](./METHODOLOGY.md) or the
[SpeedQX technical report](https://speedqx.com/how-it-works).

## License

[PolyForm Noncommercial 1.0.0](https://polyformproject.org/licenses/noncommercial/1.0.0/)
