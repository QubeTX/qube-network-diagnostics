# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

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
