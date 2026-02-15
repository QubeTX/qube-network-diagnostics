# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [2.4.1] - 2026-02-15

### Changed
- Version short flag changed from `-V` to `-v` for convenience

## [2.4.0] - 2026-02-15

### Added
- `-d, --dns` flag for standalone DNS configuration — choose a provider, set DNS, verify connectivity, auto-revert on failure, then run full diagnostics
- NextDNS support (encrypted DNS with filtering) across all platforms:
  - Windows: DNS-over-HTTPS via `netsh` DoH template registration
  - Linux: DNS-over-TLS via `systemd-resolved` config, `nmcli` fallback
  - macOS: NextDNS CLI client integration with plain IP fallback
- Extended DNS provider menu: Hybrid, Cloudflare, Google, NextDNS, Automatic (DHCP)
- JSON output for `--dns` action with structured success/failure/revert fields
- Auto-revert to DHCP when DNS verification fails after change (includes platform-specific NextDNS cleanup)
- Man-page style `--help` output with grouped sections (Modes, Output, Speed Test, Actions) and usage examples

### Fixed
- Fix `extract_stat_value` dead code warning on Linux — function is only used in Windows `#[cfg]` block

## [2.3.1] - 2026-02-15

### Fixed
- Fix CI build failure on Linux targets — `TIMEOUT_QUICK` import was removed from `dns.rs` but is used inside `#[cfg(target_os = "linux")]` block
- Fix unused import warning for `TIMEOUT_SLOW` in `stages.rs` (used only on non-Windows platforms)
- Fix unnecessary `mut` warning on `renew_cmd` in `dhcp.rs` Linux DHCP renewal

## [2.3.0] - 2026-02-15

### Added
- DNS verification after connectivity checks in all fix flow stages — resolves 3 domains to confirm DNS is actually working, not just HTTP
- DNS server selection prompt when DNS fails: Hybrid (Cloudflare + Google, recommended), Cloudflare, Google, or Automatic (DHCP)
- DNS reachability test before committing server changes — detects corporate firewalls blocking public DNS and auto-adjusts
- Stage 3 macOS: set hybrid DNS (1.1.1.1 + 8.8.8.8) immediately after network service recreation, preventing DNS gap while DHCP delivers servers

### Fixed
- Fix `--fix` on macOS reporting "connected" when DNS is broken — HTTP captive portal check passed but DNS resolution failed after `configd` restart + DHCP reset race
- Fix Stage 2 DHCP renew failing on macOS Wi-Fi — added 5-second delay after interface re-enable for Wi-Fi association before DHCP request
- Fix Stage 3 macOS creating network service without DNS servers, leaving DNS broken until DHCP delivers servers

## [2.2.1] - 2026-02-13

### Fixed
- Fix `--fix` flow on macOS failing all Stage 1 steps when run without `sudo` — elevation check now runs before any stages, exiting immediately with a clear privilege hint instead of attempting commands that require root
- Fix `--fix` JSON mode silently running stages without elevation — now returns structured `elevated_privileges_required` error
- Fix macOS DNS flush reporting failure when `killall -HUP mDNSResponder` needs root but `dscacheutil -flushcache` succeeds — now reports partial success instead of error

## [2.2.0] - 2026-02-13

### Changed
- Replace WMI adapter enumeration with native Windows APIs (`ipconfig` crate + `GetIfEntry2`)
  - ~5ms vs ~300-500ms for adapter data collection
  - Accurate adapter type classification via `PhysicalMediumType` (Wi-Fi, Ethernet, Bluetooth, WWAN) instead of generic WMI "Ethernet 802.3"
  - Precise connection status: "No Cable" vs "Disabled" vs "Down" vs "Standby" via `AdminStatus` + `MediaConnectState`
- Replace per-adapter PowerShell `Get-NetAdapter` calls with `GetIfEntry2` for instant link speed/duplex in tech mode
- Replace WMI in `--fix` adapter listing with `ipconfig` crate for faster adapter detection
- Defer WMI driver query (`Win32_PnPSignedDriver`) to tech mode only — user mode no longer pays WMI cost
- Match drivers to adapters by hardware description instead of name substring (more reliable)

### Added
- Per-adapter link speed in user-mode summary (e.g., "Wi-Fi 866 Mbps" or "Wi-Fi 2.4 Gbps")
- Tech-mode NETWORK ADAPTERS section now shows: MAC address, TX/RX link speeds, gateway, DNS servers, MTU, IPv4 metric
- New JSON fields: `description`, `mac_address`, `link_speed_mbps`, `rx_link_speed_mbps`, `dns_servers`, `gateways`, `media_connect_state`, `physical_medium`, `mtu`, `ipv4_metric`
- `--fix` default interface detection now picks adapter with lowest IPv4 metric (route preference) instead of first active

## [2.1.0] - 2026-02-13

### Added
- Show adapter type names in diagnostic summary (e.g., "2 active (Ethernet, Wi-Fi)" instead of just "2 active")
- Normalize WMI adapter types for cleaner display (e.g., "Ethernet 802.3" -> "Ethernet")
- Show fix hint ("Run 'nd300 -f' to attempt automatic fixes") when failures are detected

### Changed
- Consolidate duplicate subprocess calls in technician mode via pre-fetched `SharedCache`
  - `netstat -ano` shared across connections, listening_ports, and connection_states (was 3 calls)
  - `ipconfig /all` shared across dhcp, vpn, and ipv6 (was 2 calls)
  - `sysinfo::Networks` shared across adapter_hw_stats and traffic_counters (was 2 calls)
  - `default_net::get_default_gateway()` shared with reverse_dns (was 2 calls)
  - Saves ~200-800ms in technician mode by eliminating redundant subprocess spawns
- Label "Bluetooth Device (Personal Area Network)" as "BT PAN" instead of "Bluetooth" in adapter summary to avoid confusion with Bluetooth radio status
- Improve virtual adapter detection with additional patterns: "host-only", "vm network"

## [2.0.3] - 2026-02-12

### Fixed
- Add retry logic to post-fix connectivity checks: retry up to 3 times with 30-second countdown between each attempt, preventing false failure reports after interface resets (especially on Windows where Wi-Fi reconnection takes 15-30+ seconds)
- Replace hardcoded 2s/5s/8s single-check waits with visible spinner countdown so the tool doesn't appear frozen during reconnection

## [2.0.2] - 2026-02-11

### Fixed
- Fix Windows service restart failure: replace `net stop/start` with `sc` commands for PPL-protected services (dnscache, Dhcp), with PowerShell fallback and graceful degradation instead of hard failure
- Remove redundant dim "Checking for VPN connections..." text that leaked raw ANSI escape codes (SGR 2 unsupported on some Windows terminals); VPN spinner already covers this

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
