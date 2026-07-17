# ND300 — Network Diagnostic 300

## Project Plan & Developer Specification

**Product Line:** QubeTX Developer Tools (300 Series)
**Sibling Tool:** TR-300 (Machine Report), SD-300 (System Diagnostic)
**Language:** Rust
**License:** PolyForm Noncommercial (consistent with TR-300)
**Target Platforms:** Windows, macOS, Linux — two standalone binaries per platform (`nd300` + `speedqx`), with no admin/root required for core diagnostics

---

## Current Shipping Status (v3.6.0 release candidate, updated 2026-07-17)

This file began as the original product plan. The implemented package now ships as the lowercase crates.io package `nd300` while keeping ND300 as the product brand. End users should install with `cargo install nd300` or the GitHub release installers (`nd300-installer.sh`, `nd300-installer.ps1`, Windows MSI/archives). GitHub releases also publish legacy `nd-300-installer.*` aliases so older installed copies can still self-update.

The manifest is staged at `3.6.0`; the last published baseline is `3.5.2`.
Release is a two-push, exact-SHA flow: merge through cross-platform `main` CI so
the crate publisher handles crates.io, then push the immutable version tag for
cargo-dist hosting and the chained Windows installers. A release is not complete
until Apple accepts both Mac architectures, the post-signing artifacts and later
Windows assets are attested, and all 28 assets are verified.

The remaining release and follow-up work is tracked in root `TASKS.md`, with
Windows-ready commands plus the rationale, user impact, and acceptance evidence
for every item. Update that board rather than relying on this Mac session.

`nd300 update` remains Cargo-first but is bounded and never runs an unsolicited
Rust-toolchain update. On macOS/Linux its fallback downloads the exact-tag target
archive plus required checksum, extracts only the two allowlisted programs,
verifies both versions and Mac trust, and installs the pair transactionally with
rollback. Windows retains the four installer-aware paths. Unix uninstall is
origin-aware and refuses unknown or package-manager-owned locations.

## 1. Overview

ND300 is a zero-config, cross-platform CLI network diagnostic tool. It must work identically in its core diagnostics across Windows, macOS, and Linux, while also leveraging platform-specific APIs to surface additional OS-native diagnostic information (such as driver status on Windows, rfkill state on Linux, or firewall configuration on macOS). The tool should detect the host OS at runtime and silently include or exclude platform-specific checks as appropriate — the user never needs to know or care about the platform logic. It runs a comprehensive battery of network tests and presents results in two tiers:

1. **A user-facing diagnostic summary** at the top of the output — clear pass/warn/fail verdicts on major categories that any user (including non-technical and gaming users) can understand at a glance.
2. **A detailed technical report** below the summary — an ASCII/Unicode table with raw diagnostic data for technicians and power users.

The tool should feel like a natural extension of the TR-300 ecosystem: same visual language (Unicode box-drawing with ASCII fallback), same `--json` output for scripting, and self-contained release binaries.

ND300 is intended to be a **one-shot diagnostic**, not a live monitor. The user runs `nd300`, it executes all tests (including a timed speed test), and outputs the full report. Total runtime target: **15–25 seconds** depending on network conditions, with the speed test being the primary time cost.

---

## 2. Diagnostic Tests — What ND300 Should Run

### 2.1 Network Interface Detection

- Build one topology snapshot from the default route, current kernel flags,
  hardware-port/network-service mapping, and addresses.
- Count only usable physical uplinks as active. Keep loopback, AWDL/LLW,
  virtual adapters, dormant tunnels, and link-local-only devices in the detail
  inventory without inflating the headline.
- For each interface: name, type (wired/wireless/virtual), operational state,
  MAC address, assigned local/private IP, and subnet mask.
- On Wi-Fi interfaces (where OS APIs permit): SSID, signal strength (dBm and/or percentage), frequency band (2.4 GHz / 5 GHz / 6 GHz), channel number, and link speed.
- Identify the primary/default interface. If a VPN owns the default route,
  report the logical tunnel and underlying physical uplink separately.

### 2.1.1 Network Driver & Adapter Status

ND300 should scan and report the status of network drivers and network-related software/services. This helps diagnose root causes when connectivity issues stem from broken, disabled, or missing drivers rather than the network itself.

**What to scan:**

- **Network adapters (all platforms):** Enumerate all network adapters present on the system (not just active ones). For each adapter, report:
  - Adapter name and type (Ethernet, Wi-Fi, Bluetooth PAN, virtual/VPN adapters, etc.)
  - Current status: **Active** (connected and passing traffic), **Enabled but disconnected** (driver loaded, no link), **Disabled** (adapter manually or system-disabled), **Error/Malfunctioning** (driver loaded but reporting errors).
  - Whether the adapter has a valid IP assignment or is unconfigured.

- **Windows-specific driver scanning:**
  - Query the Windows Driver Store / WMI (`Win32_NetworkAdapter`, `Win32_NetworkAdapterConfiguration`, `Win32_PnPEntity`) to retrieve:
    - Driver name and version for each network adapter.
    - Driver status (functioning, degraded, error, disabled).
    - Driver date (how old the installed driver is).
    - Whether the device has a problem code (Device Manager yellow/red icon equivalent).
  - Scan for known network-related services and their state:
    - **DHCP Client service** — running / stopped.
    - **DNS Client service** — running / stopped.
    - **WLAN AutoConfig** (Wi-Fi service) — running / stopped / not installed.
    - **Network Location Awareness (NLA)** — running / stopped.
    - **Windows Network Connection Broker** — running / stopped.
  - Report any disabled or errored adapters prominently — these are common causes of "no internet" that users don't realize.

- **macOS-specific scanning:**
  - macOS does not have a traditional "driver" model like Windows. Instead, scan for:
    - Network service ordering (via `networksetup -listnetworkserviceorder` equivalent or System Configuration framework).
    - Active vs. inactive network services and their assigned interfaces.
    - Whether Wi-Fi is powered on or off (CWWiFiClient / CoreWLAN).
    - Whether a VPN or proxy configuration is active that may affect connectivity.
    - Firewall status (Application Firewall enabled/disabled, stealth mode).
    - Whether any kernel extensions (kexts) or system extensions related to networking are loaded (useful for diagnosing third-party VPN/firewall interference).

- **Linux-specific scanning:**
  - Query for network-related kernel modules and their status:
    - Key modules like `e1000e`, `iwlwifi`, `rtl8xxxu`, `ath9k`, `r8169`, etc. — report whether the expected driver module for each detected adapter is loaded.
    - If an adapter is detected via PCI/USB bus but has no loaded driver module, flag it as **Driver missing**.
  - Check the status of critical networking services/daemons:
    - **NetworkManager** — running / stopped / not installed.
    - **systemd-networkd** — running / stopped / not installed.
    - **systemd-resolved** — running / stopped.
    - **wpa_supplicant** (Wi-Fi authentication) — running / stopped.
    - **dhclient / dhcpcd** — running / stopped.
  - Report any network interfaces in a `DOWN` state at the kernel level vs. administratively disabled.
  - Detect if `rfkill` is blocking wireless interfaces (hard block vs. soft block).

**Presentation in the diagnostic summary:**

- Add a new summary line for adapter/driver health. Examples:
  - `✓ Adapters       All network adapters healthy`
  - `⚠ Adapters       1 adapter disabled (Ethernet), Wi-Fi OK`
  - `✗ Adapters       Wi-Fi adapter driver error — may need reinstall`
  - On macOS/Linux where "drivers" aren't the framing: `✓ Adapters       All network services active`

**Presentation in the detailed report:**

- A dedicated table in the detailed technical section listing every detected adapter, its type, status, and (on Windows) driver version/date. On Linux, show the kernel module name. On macOS, show the network service name and assigned interface.
- A separate table or subsection for network service/daemon status (OS-appropriate).

**Important:** The driver/adapter scan is a lightweight, read-only operation. It should add negligible time to the overall diagnostic run. It does NOT install, update, or modify any drivers — it only reports what is present and whether it appears to be working.

### 2.2 Gateway & Routing

- Detect the default gateway IP address.
- Ping the default gateway and report latency (min/avg/max over a small sample, e.g., 5 pings).
- Report whether the gateway is reachable (pass/fail).
- Detect any secondary gateways or routing anomalies if feasible.

### 2.3 DNS Diagnostics

- Detect configured DNS servers (from OS configuration).
- For each DNS server: ping latency and whether it responds to a test lookup.
- Perform a DNS resolution test against a known domain (e.g., `dns.google`, `cloudflare.com`, or `one.one.one.one`) using each configured DNS server.
- Measure DNS resolution time.
- Flag if DNS servers are unreachable or slow (e.g., resolution > 200ms as a warning, > 500ms or failure as an error).
- Detect DNS-over-HTTPS or DNS-over-TLS configuration if discoverable.
- Optionally detect DNS poisoning/hijacking by comparing resolution results across servers (advanced — nice to have).

### 2.4 Public IP & Geolocation

- Determine the public/external IP address using a lightweight HTTP check (e.g., Cloudflare's `https://1.1.1.1/cdn-cgi/trace`, or `https://ifconfig.me`, or `https://api.ipify.org`).
- If the public IP lookup succeeds, perform a basic geolocation lookup (city, region, country, ISP/ASN). Free APIs like `ip-api.com` or similar can be used. Cache or bundle a lightweight lookup if preferred to avoid API dependency.
- Report NAT type if determinable (whether the local IP matches the public IP, i.e., the machine is directly exposed vs. behind NAT).

### 2.5 Latency Tests

- Ping a set of well-known, highly available endpoints and report latency for each:
  - **Cloudflare:** `1.1.1.1`
  - **Google DNS:** `8.8.8.8`
  - **A regional/general endpoint:** e.g., `ping.archlinux.org` or another neutral, reliable host.
- For each: min/avg/max/jitter over a small sample (5–10 pings).
- Optionally, a traceroute-style hop count to one of these endpoints (nice to have, can be complex cross-platform).

### 2.6 Speed Test

This is a core differentiator. ND300 should run a brief, automated throughput test. **Target duration: 10–15 seconds.**

**How speed tests work at a high level:**

Speed tests measure throughput by transferring data to/from a known server and measuring how much data moves in a given time window. The simplest approach:

- **Download test:** Make an HTTP(S) GET request to a server that serves a large payload (or a stream of data) and measure bytes received over time.
- **Upload test:** Make an HTTP(S) POST request sending a large payload and measure bytes sent over time.

**Recommended approach for ND300:**

- Use **Cloudflare's speed test infrastructure**. Cloudflare exposes endpoints that can be used for this:
  - Download: `https://speed.cloudflare.com/__down?bytes=<N>` — returns `N` bytes of random data.
  - Upload: `https://speed.cloudflare.com/__up` — accepts POST data and returns timing info.
- Alternatively, use a simple large-file download from a CDN with known high bandwidth (e.g., download a 10MB or 25MB test payload from Cloudflare or a similar provider).
- Run the download test for ~5-8 seconds (multiple sequential or parallel requests if needed to saturate the connection), measure total bytes transferred, compute Mbps.
- Run the upload test for ~5-7 seconds using the same approach.
- Report: **download speed (Mbps)**, **upload speed (Mbps)**, and optionally **loaded latency** (latency measured during the speed test, which reveals bufferbloat).

**Important considerations:**
- The speed test should be clearly communicated (since it transfers real data). It runs by default, and the implemented skip flag is `--fast`.
- Display a progress indicator during the speed test (e.g., `Running speed test... [=====>    ] 7s / 15s`).
- If the speed test endpoints are unreachable (e.g., corporate firewall), fail gracefully and report that the speed test could not be completed.

### 2.7 Port & Connectivity Checks

- Test outbound connectivity on common ports to verify nothing is blocked:
  - HTTP (80), HTTPS (443), DNS (53), SSH (22).
- For each: attempt a TCP connection to a known server on that port and report success/failure/timeout.
- This helps diagnose firewall or ISP-level blocking issues.

### 2.8 Additional Checks (Nice-to-Have / Stretch Goals)

- **IPv6 connectivity:** Detect if IPv6 is available, test connectivity.
- **MTU detection:** Determine the path MTU to avoid fragmentation issues (useful for gaming users).
- **Bufferbloat detection:** Compare unloaded latency (from ping tests) to loaded latency (during speed test). If loaded latency is significantly higher, flag bufferbloat.
- **Captive portal detection:** Attempt to detect if the network has a captive portal (common in hotels, airports).

---

## 3. Output Design

### 3.1 User-Facing Diagnostic Summary (Top Section)

This is the first thing the user sees. It should be a clean, scannable block of verdicts. Each major category gets a one-line status with a clear icon/label.

**Status levels:**
- ✓ OK (or `[OK]` in ASCII mode) — green if color is enabled
- ⚠ WARN — yellow
- ✗ FAIL — red
- — SKIP — gray/dim (if a test was skipped, e.g., speed test with `--fast`)

**Example output (conceptual, not final design):**

```
+==================================================+
|         ND-300 NETWORK DIAGNOSTIC                |
|         QubeTX Developer Tools                   |
+==================================================+
|                                                  |
|  DIAGNOSTIC SUMMARY                              |
|                                                  |
|  ✓ Adapters         All network adapters healthy  |
|  ✓ Network          Connected via Wi-Fi (5 GHz)  |
|  ✓ Gateway          Reachable (1ms)              |
|  ✓ DNS              Resolving normally (12ms)    |
|  ✓ Internet         Public IP obtained           |
|  ⚠ Speed            Download OK, Upload slow     |
|  ✓ Ports            All common ports open        |
|                                                  |
+==================================================+
```

**Key principles for the summary:**
- Use natural language that any user would understand. "Connected via Wi-Fi (5 GHz)" not "wlan0 UP 802.11ac".
- The short description after each verdict should give just enough context. "Resolving normally (12ms)" tells the user DNS is fine. "Resolving slowly (487ms)" tells them something is off.
- For speed: translate Mbps into relatable language. E.g., "120 Mbps down / 15 Mbps up" but also consider contextual labels like "Good for streaming and gaming" or "Upload may be slow for video calls." This is the user-facing section — be helpful, not just numerical.
- For warnings and failures: briefly state what's wrong in plain language. "Upload speed below 5 Mbps — video calls may struggle" or "DNS server at 192.168.1.1 not responding — using fallback."

### 3.2 Detailed Technical Report (Bottom Section)

Below the summary, present the full diagnostic data in Unicode/ASCII tables (consistent with TR-300's visual style).

**This section includes:**
- **Network adapter inventory** — All detected adapters (not just active ones), their type, status (active/enabled-disconnected/disabled/error), and IP assignment status. On Windows: driver name, driver version, driver date, device problem codes. On Linux: kernel module name and load status. On macOS: network service name and ordering.
- **Network services/daemons status** — OS-appropriate table of critical network services. Windows: DHCP Client, DNS Client, WLAN AutoConfig, NLA, etc. Linux: NetworkManager, systemd-networkd, wpa_supplicant, etc. macOS: VPN/proxy configurations, firewall status, relevant system extensions.
- **Interface details table** — All active interfaces, IPs, MACs, Wi-Fi details.
- **Gateway & routing table.**
- **DNS server table** — Each server, its latency, resolution test result.
- **Public IP and geolocation info.**
- **Latency table** — Each endpoint, min/avg/max/jitter.
- **Speed test results** — Download Mbps, upload Mbps, loaded latency, test duration, test server.
- **Port connectivity table.**
- **Platform-specific extras** — Any additional OS-specific findings (rfkill blocks on Linux, firewall status on macOS, disabled adapters on Windows, etc.).
- **Any warnings or errors with technical detail.**

The detailed section should use the same Unicode box-drawing style as TR-300, with consistent column alignment and clean formatting.

### 3.3 JSON Output

`nd300 --json` should output a structured JSON object containing all diagnostic data, suitable for scripting, logging, piping to monitoring systems, or integration with other tools.

The JSON schema should be well-documented and stable across versions.

---

## 4. CLI Interface

### 4.1 Commands & Flags

| Command / Flag           | Description                                                |
|--------------------------|------------------------------------------------------------|
| `nd300`                  | Run full diagnostic (default)                              |
| `nd300 --help`           | Show help text                                             |
| `nd300 --version`        | Show version                                               |
| `nd300 --json`           | Output results as JSON                                     |
| `nd300 --ascii`          | Force ASCII mode (no Unicode box-drawing)                  |
| `nd300 --no-color`       | Disable colored output                                     |
| `nd300 --fast`           | Skip the speed test (faster execution)                     |
| `nd300 --speed-duration`  | Set speed test duration in seconds (default: 10)           |
| `nd300 -t, --tech`       | Technician mode                                            |
| `nd300 -T, --title`      | Custom title (consistent with TR-300)                      |
| `nd300 --verbose`        | Show additional debug/trace information during execution    |
| `nd300 fix` / `nd300 -f` | Diagnostic-driven triage and recovery loop                 |
| `nd300 update`           | Self-update to the latest release                          |
| `nd300 clear-dns` / `-c` | Flush the DNS cache and exit                               |
| `nd300 uninstall`        | Remove `nd300` and `speedqx` from this system              |

### 4.2 Exit Codes

- `0` — All diagnostics passed (OK).
- `1` — One or more diagnostics returned WARN.
- `2` — One or more diagnostics returned FAIL.
- Non-zero exit codes allow ND300 to be used in scripts and CI (e.g., "fail the build if the network is broken").

---

## 5. Technical Implementation Notes

### 5.1 Rust Crates to Evaluate

These are starting suggestions — the developer should evaluate and choose:

- **`sysinfo`** — Cross-platform system/network interface info. Already likely used by TR-300.
- **`pnet` / `pnet_datalink`** — Low-level network packet construction. Useful for ICMP ping implementation.
- **`surge-ping`** — Pure Rust ICMP ping library (cross-platform). Good for the latency tests.
- **`dns-lookup`** or **`trust-dns-resolver`** (now `hickory-resolver`) — DNS resolution and diagnostics.
- **`reqwest`** — HTTP client for speed test, public IP lookup, and geolocation API calls. Async with `tokio`.
- **`tokio`** — Async runtime. Useful for running multiple tests concurrently (e.g., pinging multiple endpoints in parallel).
- **`indicatif`** — Terminal progress bars. For the speed test progress indicator.
- **`comfy-table`** or **`tabled`** — Table formatting for the detailed output section.
- **`colored` / `owo-colors`** — Terminal color output.
- **`clap`** — CLI argument parsing (consistent with TR-300 if it uses this).
- **`serde` + `serde_json`** — JSON serialization for `--json` output.
- **`default-net`** — Cross-platform default gateway and interface detection.
- **`local-ip-address`** — Simple local IP detection.
- **`wifi-rs`** or platform-specific APIs — Wi-Fi info (SSID, signal strength). This may require platform-specific code paths.
- **`wmi`** (Windows only) — WMI queries for driver info, adapter status, service state. Compile-gated with `#[cfg(target_os = "windows")]`.
- **`windows` or `winapi`** (Windows only) — Low-level Windows API access for network adapter enumeration, service queries, and device status.
- **macOS Wi-Fi collection** — core mode uses the cached topology only;
  technician mode runs bounded `system_profiler SPAirPortDataType` JSON with a
  text fallback for older supported macOS versions. Do not use the removed
  private `airport` utility.
- **`neli`** or **`netlink-sys`** (Linux only) — Netlink socket interface for nl80211 Wi-Fi queries, route information, and kernel interface state.
- **`nix`** (Linux/macOS) — Unix system call wrappers, useful for interface ioctl calls, rfkill queries, and other low-level operations.

### 5.2 Cross-Platform Considerations

ND300 must be fully cross-platform (Windows, macOS, Linux) and build the self-contained `nd300` and `speedqx` binaries on every release platform. However, the diagnostics available differ by OS. Compile-time platform branches run the appropriate checks; features that do not apply should be silently omitted (not shown as "SKIP" or "N/A" unless the user would expect them to be present).

**General cross-platform rules:**

- Every diagnostic section should have a platform-agnostic core (interface detection, ping, DNS, speed test, port checks) that works identically on all three platforms.
- Platform-specific extras are additive — they provide additional insight on the platforms where they're available.
- If a diagnostic cannot run on a given platform, it should degrade gracefully: either omit the row/section entirely, or show a brief explanation ("Detailed driver info available on Windows only" or similar — but only if the section header is already being shown).

**Platform-specific behaviors:**

#### Windows
- **Driver scanning:** Full driver enumeration via WMI — driver name, version, date, problem codes for all network adapters. This is a major diagnostic feature on Windows.
- **Network services:** Scan DHCP Client, DNS Client, WLAN AutoConfig, NLA, Network Connection Broker service states.
- **Wi-Fi info:** `netsh wlan show interfaces` or the Native Wi-Fi API.
- **ICMP ping:** Raw sockets typically work without elevation on Windows 10+.

#### macOS
- **No traditional driver scanning.** macOS manages drivers through kernel extensions (kexts) and system extensions. Instead:
  - Report network service ordering and active/inactive services (System Configuration framework).
  - Report Wi-Fi power state and adapter details (CoreWLAN / CWWiFiClient).
  - Detect active VPN or proxy configurations.
  - Report Application Firewall status (enabled/disabled, stealth mode).
  - Detect loaded networking-related system extensions that may interfere (e.g., third-party VPN or firewall extensions).
- **Wi-Fi info:** bounded `system_profiler` in technician mode only. Parse only
  the connected network, treat redacted SSIDs as unavailable, and trust the
  reported band rather than channel-number guesses.
- **ICMP ping:** Requires no special privileges on macOS.

#### Linux
- **Driver/module scanning:** Check for loaded kernel modules for each detected network adapter. Flag adapters that appear on the PCI/USB bus but lack a loaded driver module.
- **Network services:** Check status of NetworkManager, systemd-networkd, systemd-resolved, wpa_supplicant, dhclient/dhcpcd.
- **rfkill:** Detect if `rfkill` is blocking wireless interfaces (hard block vs. soft block) — a common and confusing cause of Wi-Fi failure on Linux.
- **Wi-Fi info:** `iw` / nl80211 netlink interface, or fallback to parsing `iwconfig`.
- **ICMP ping:** Often requires elevated privileges (raw sockets). Consider using the `setcap` approach (`setcap cap_net_raw+ep nd300`), or fall back to spawning the system `ping` command if raw socket access is denied. Document this.
- **Firewall:** Optionally detect whether `iptables`/`nftables`/`ufw`/`firewalld` has rules that might block outbound traffic on tested ports.

**Speed test** should work anywhere with HTTPS outbound access. Cloudflare endpoints are globally available.

**Abstraction pattern:** The developer should abstract platform-specific behavior behind a Rust trait (e.g., `trait PlatformNetworkInfo`) with separate implementations for each OS, selected at compile time via `#[cfg(target_os = "...")]` or at runtime via `std::env::consts::OS`. This keeps the main diagnostic logic clean while supporting platform divergence in the implementations.

### 5.3 Execution Flow

1. Print header/banner.
2. Run all non-speed-test diagnostics concurrently where possible (interface detection, gateway ping, DNS tests, public IP lookup, latency tests, port checks). Display a spinner or progress indicator.
3. Run the speed test (sequential by nature, with its own progress bar).
4. Compile results.
5. Print the user-facing summary.
6. Print the detailed technical report.
7. Exit with appropriate code.

**Target runtime:** bounded by the diagnostic wall-clock cap. Core work should
return promptly; technician mode intentionally takes longer because it includes
route, sustained-loss, path-MTU, and loaded-latency phases. Partial results must
render when the cap is reached, and timed-out child processes must be killed and
reaped.

### 5.4 Repair Safety Invariants

- macOS repair must never delete/recreate a network service, retrieve a Wi-Fi
  password, or infer state from an incomplete snapshot.
- The high-risk macOS replacement may only cycle one unambiguously mapped,
  enabled physical service. Register re-enable before disable, preserve DNS
  servers and search domains exactly, and resolve the restore token only after
  enabled state plus the expected physical route/reachability are verified.
- Keep the production service-cycle gate false until the exact production path
  passes under an independent offline watchdog on a disposable Mac VM/service.
  Never test it on a computer whose Codex, SSH, or other recovery control uses
  the link being cycled.
- DHCP renewal is allowed only when the selected service is already configured
  for DHCP. Static, Manual, BootP, disabled, virtual, missing, or ambiguous
  configurations must not be mutated.
- Restore operations run LIFO after Ctrl-C, SIGTERM, SIGHUP, panic, timeout, or
  normal completion. Register every consumer-VPN inverse before disconnect;
  enterprise VPNs remain untouched. Report `reverted: true` only after proof.
- Elevated Unix operations must resolve the invoking user from numeric account
  data, drop privileges for user-scoped commands, and create private atomic
  reports owned by that user rather than root.

---

## 6. Distribution & Installation

Consistent with TR-300:

- **GitHub Releases** with prebuilt binaries for Windows (x86_64), macOS (x86_64, aarch64), Linux (x86_64, aarch64).
- **Shell installer:** `curl --proto '=https' --tlsv1.2 -LsSf https://github.com/QubeTX/qube-network-diagnostics/releases/latest/download/nd300-installer.sh | sh` for macOS/Linux.
- **PowerShell installer:** `powershell -ExecutionPolicy Bypass -c "irm https://github.com/QubeTX/qube-network-diagnostics/releases/latest/download/nd300-installer.ps1 | iex"` for Windows.
- **Cargo install:** `cargo install nd300` for Rust developers.
- **Legacy updater aliases:** `nd-300-installer.sh` and `nd-300-installer.ps1` are uploaded to every release for older installed copies.
- **Self-update distinction:** the shell command above is a user-invoked fresh
  install path. The built-in Unix updater must not execute downloaded scripts;
  it uses the verified exact-tag native archive transaction described above.
- **Mac trust:** both `nd300` and `speedqx`, in arm64 and Intel archives, require
  the pinned Developer ID identity, hardened runtime, timestamp, and Apple
  notarization status `Accepted`; there is no unsigned publishing fallback.
- **Attestations and permissions:** attest final post-signing cargo-dist assets
  and later Windows add-ons separately; workflow defaults are read-only.
- **Man pages:** include `man/` in archives. macOS instructions use a
  user-writable man directory and do not require `mandb`.

---

## 7. Branding & Visual Consistency

- The output header should say `ND-300 NETWORK DIAGNOSTIC` and `QUBETX DEVELOPER TOOLS`, matching TR-300's style.
- Use the same Unicode box-drawing characters and the same table styling conventions as TR-300.
- Color scheme should be consistent: green for OK, yellow for WARN, red for FAIL, with `--no-color` to disable.
- The landing page (nd300.emmetts.dev or nd300.qubetx.com) should follow the same design template as tr300.emmetts.dev.

---

## 8. Summary of Deliverables

1. **Rust binaries** — `nd300` and `speedqx` — cross-platform release pair.
2. **GitHub repository** — under the QubeTX organization, with README, LICENSE (PolyForm Noncommercial), CI/CD for release builds.
3. **Shell installer script** — consistent with TR-300's installation method.
4. **JSON output schema** — documented in the README.
5. **Landing page** — product page matching the TR-300 site design.

---

## 9. Non-Goals (Out of Scope)

- ND300 is **not** a TUI or live monitor. It's a one-shot diagnostic tool. (SD-300 handles the live monitoring role.)
- ND300 is **not** a full-featured speed test app (like Ookla Speedtest CLI). The speed test is a diagnostic component, not a benchmark.
- ND300 does **not** require an account, API key, or internet connection to run basic diagnostics (speed test and public IP obviously require internet, but interface/gateway/DNS config checks should work offline).
