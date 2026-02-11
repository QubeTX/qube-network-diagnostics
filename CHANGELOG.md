# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

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
