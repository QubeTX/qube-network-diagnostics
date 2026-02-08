# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

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
