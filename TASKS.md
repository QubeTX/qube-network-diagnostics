# ND300 v3.6.0 Handoff Board

_Updated 2026-07-17. This file is intentionally tracked at the repository root
so the remaining work is visible from Windows Explorer, GitHub, and a fresh
clone without access to the Mac used for this hardening pass._

## Status and non-negotiable safety rules

- Source branch: `codex/nd300-macos-hardening-v3.6.0`
- Release candidate: `3.6.0`; last published version: `3.5.2`
- `P0` means required before calling v3.6.0 released. `P1` is recommended
  follow-up. `P2` is useful cleanup or additional evidence.
- Do not enable `MACOS_SERVICE_CYCLE_VALIDATED`. The replacement service cycle
  is implemented and fixture/fault tested, but production remains deliberately
  unavailable until a disposable Mac service passes the full destructive
  matrix. Shipping it unavailable is safe and permitted.
- Do not run `nd300 fix`, a service cycle, or any network-mutating command on a
  machine that supplies the only Codex/SSH/remote-control path.
- Do not export, print, download, or recommit Apple credentials. The five Apple
  secrets and two repository variables needed by GitHub Actions are already
  configured. The temporary local App Store Connect key was securely removed.
- Never reuse or move a release tag. Never publish locally merely because a
  token is available. Merge through exact-SHA CI, let the crate workflow
  publish, and then create one immutable tag.

## Windows quick start

Run these in PowerShell from a clean clone on the Alienware. They are read-only
with respect to network configuration.

```powershell
git fetch origin
git switch codex/nd300-macos-hardening-v3.6.0
git pull --ff-only
rustup toolchain install 1.97.0
cargo +1.97.0 fmt --all -- --check
cargo +1.97.0 clippy --locked --workspace --all-targets -- -D warnings
cargo +1.97.0 test --locked --workspace --all-targets
cargo +1.97.0 build --release --locked
.\target\release\nd300.exe --version
.\target\release\speedqx.exe --version
```

Expected version for both binaries: `3.6.0`.

## P0 — release-critical work that can be completed from Windows

### ND36-001 — Merge only after exact-SHA cross-platform CI

**Status:** Pending hosted evidence.

**Why needed/recommended:** The local Mac proves only one host/toolchain shape.
ND300 contains extensive `cfg`-gated Windows, Linux, Intel Mac, and ARM Mac
code. A clean local build cannot catch target-only unused code, imports,
installer drift, deployment-floor mistakes, or runner-specific shell behavior.

**Impact:** Completing this gate materially reduces the chance that v3.6.0
works on the audit Mac but fails to compile or test on the Alienware or another
published target. Skipping it risks publishing an immutable broken release.

**Work from Windows:**

1. Push/update the source branch and open the PR to the repository default
   branch (`main`).
2. Require `ci.yml` to pass on the exact PR head SHA across macOS, Linux, and
   Windows. Do not rely on a green run for an older commit.
3. Merge normally; do not force-push or bypass checks.
4. Confirm the `CI` run on the resulting `main` SHA succeeds and the
   `crates-publish.yml` run publishes `nd300 3.6.0` (or reports that this exact
   version is already published).

**Acceptance:** The merged `main` SHA equals the reviewed source SHA or a
documented GitHub merge SHA containing it; all required jobs are green; the
crate page reports 3.6.0; no release tag has been pushed early.

### ND36-002 — Run the native Alienware diagnostic smoke

**Status:** Pending physical Windows hardware.

**Why needed/recommended:** Windows commonly exposes a default adapter by GUID
or `AdapterName` while `sysinfo` exposes its friendly name. VPNs, Hyper-V,
WSL, Bluetooth, Docker, and gaming/VPN software add virtual adapters that can
look active. Fixtures cover these shapes, but only the user's real Alienware
can prove that the headline count, primary adapter, routes, listeners, IPv6,
and VPN classification agree on that machine.

**Impact:** Completing this catches user-visible false interface/VPN counts and
Windows-only parser/runtime failures before they affect the user's main
computer. If deferred, the release is still protected by tests and hosted CI,
but real-world diagnostic accuracy on the primary machine remains unproven.

**Work from Windows (non-elevated PowerShell):**

```powershell
New-Item -ItemType Directory -Force .\artifacts\alienware-smoke | Out-Null
Get-NetAdapter | Format-Table -AutoSize | Out-File .\artifacts\alienware-smoke\net-adapters.txt
Get-NetRoute -DestinationPrefix "0.0.0.0/0" | Format-Table -AutoSize | Out-File .\artifacts\alienware-smoke\default-routes.txt
.\target\release\nd300.exe --fast --json | Tee-Object .\artifacts\alienware-smoke\core.json
.\target\release\nd300.exe -t --fast --json | Tee-Object .\artifacts\alienware-smoke\technician.json
.\target\release\nd300.exe --fast --ascii | Tee-Object .\artifacts\alienware-smoke\table.txt
.\target\release\speedqx.exe --version | Tee-Object .\artifacts\alienware-smoke\speedqx-version.txt
```

Do not commit the generated machine/network reports; inspect them locally and
attach redacted copies to an issue only if a defect is found.

**Acceptance:** Both binaries report 3.6.0; all runs terminate within their
documented cap; the physical active/default adapter is correctly named; dormant
or virtual adapters do not inflate the active headline; a real connected VPN is
shown separately from its carrier; no secret SSID, key, or credential appears;
and no network state changes.

### ND36-003 — Exercise the four Windows install/update origins

**Status:** Pending public v3.6.0 artifacts.

**Why needed/recommended:** Windows intentionally keeps four installer-aware
paths (Global MSI, Corporate MSI, Global EXE, Corporate EXE) plus Cargo. The
v3.6.0 work changes shared release trust and updater orchestration while
preserving those paths. Hosted builds prove creation, but upgrade testing is
needed to prove origin detection, paired `nd300`/`speedqx` versions, PATH
behavior, and cleanup from a real v3.5.2 installation.

**Impact:** Completing this prevents a successful-looking update from leaving
two versions on PATH, mismatching the two binaries, or damaging an install from
another package owner. If skipped, new installs may still work while upgrades
from the user's existing copy fail.

**Work:** In disposable Windows VMs/snapshots, test each installer origin and a
Cargo install. For each origin: install v3.5.2, record `Get-Command nd300` and
`Get-Command speedqx`, run `nd300 update`, open a fresh shell, and verify both
commands resolve to the intended v3.6.0 location. Also verify `nd300 uninstall`
removes only the proven origin. Exercise `migrate-cleanup --dry-run --json`
before any real cleanup.

**Acceptance:** Both binaries are 3.6.0 from one intended origin in a fresh
shell; the other edition and Cargo collision behavior match policy; unknown or
package-manager-owned paths are refused; uninstall never deletes an unrelated
directory; all installer sidecars verify.

### ND36-004 — Trigger and verify the fully automated release

**Status:** Pending ND36-001. No local Mac is required.

**Why needed/recommended:** The public v3.5.2 Mac artifacts failed normal
Gatekeeper trust. v3.6.0 moves signing, notarization, exact-byte repacking,
checksum regeneration, public quarantine assessment, and attestation into
GitHub Actions because this Mac will not remain available.

**Impact:** Completing this gives Mac users Developer ID-signed, Apple-accepted
artifacts and gives every platform verifiable checksums/attestations. Skipping
or bypassing it would recreate the exact distribution-trust defect this release
is meant to fix.

**Work from Windows after ND36-001 is green:**

```powershell
git fetch origin main
$releaseSha = (git rev-parse origin/main).Trim()
git tag v3.6.0 $releaseSha
git push origin v3.6.0
gh run list --workflow release.yml --limit 5
```

Watch `release.yml` to completion, then the chained
`windows-installers.yml`. The workflows—not the Windows workstation—perform
both Mac builds, ephemeral-keychain signing, Apple notarization, public
Gatekeeper checks, Windows installer construction, checksum verification, and
attestation.

**Acceptance:** Apple reports `Accepted` for ARM and Intel; both public Mac
archives pass quarantine/Gatekeeper validation; the release contains exactly
28 expected assets; every sidecar and aggregate checksum matches; GitHub
attestations verify against the release tag's source digest; both public
binaries report 3.6.0; and no unsigned fallback was hosted.

### ND36-005 — Close release documentation with evidence, not assumptions

**Status:** Pending ND36-001 through ND36-004.

**Why needed/recommended:** The current changelogs correctly call 3.6.0 a
release candidate and enumerate unproven gates. Once hosted work completes,
leaving those statements unchanged makes future maintainers unable to tell
what was actually tested, while prematurely editing them would misrepresent
the release.

**Impact:** An evidence-based closeout makes incident response and the next
release safer. It separates local green tests, hosted exact-SHA CI, Apple
acceptance, public artifacts, and physical Alienware validation.

**Work:** Record run URLs/IDs, exact release SHA, crates.io result, Apple
acceptance, 28-asset inventory, attestation result, and Alienware smoke outcome
in `CHANGELOG.md`, `HUMAN_CHANGELOG.md`, `CODEX_PROJECT.md`, and this board.
Move completed items to a dated completed section; do not erase rationale.

**Acceptance:** Repository docs make no pending claim that has completed and no
completed claim lacking an evidence link or reproducible command/result.

## P1 — recommended engineering follow-up from any machine

### ND36-006 — Share one immutable topology snapshot across diagnostics

**Status:** Deferred follow-up; current per-module semantics are fixed and
fixture-tested.

**Why needed/recommended:** `interfaces.rs` now builds a coherent internal
snapshot and the VPN logic independently correlates routes, addresses,
`scutil --nwi`, and configured VPNs. They do not yet consume one immutable
cross-module object. If the route or VPN changes between those collections,
the interface section and VPN section can briefly disagree even though each
individual parser is correct.

**Impact:** Implementing this improves cross-section consistency on Windows,
macOS, and Linux—especially during VPN connect/disconnect and DHCP transitions.
Leaving it deferred is non-destructive, but rare transient reports can still
show a tunnel/default-route state from different instants.

**Work:** Introduce a shared `TopologySnapshot` owned by the diagnostic run and
thread it through interface, VPN, route, DHCP, and Wi-Fi consumers. Preserve
serialized shapes and existing action IDs. Add injected snapshots for Windows
GUID/friendly-name adapters, default-route VPN plus physical carrier, renamed
Mac services, Linux bridges, dormant tunnels, and a simulated mid-run route
change.

**Acceptance:** All relevant sections are derived from the same timestamped
snapshot; no consumer invokes a second topology command; fixtures prove the VPN
and carrier are consistent; core mode remains bounded and does not wait for
optional Wi-Fi metadata.

### ND36-007 — Add hosted updater transaction fault injection on Unix

**Status:** Core Rust fault tests implemented; broader process/filesystem matrix
recommended.

**Why needed/recommended:** The updater now pins the destination directory,
uses descriptor-relative no-follow operations, validates two exact binaries,
and rolls back them as a pair. More hostile filesystem/process tests would
exercise crash timing, permission changes, directory replacement attempts, and
interruption between the two swaps on multiple Unix filesystems.

**Impact:** This further reduces the chance that a power loss, endpoint policy,
or malicious local race leaves only one binary updated. Existing code already
fails closed; this task increases assurance rather than fixing a known live
defect.

**Work:** Add deterministic fault points before/after backup, each rename,
version validation, rollback, and cleanup. Test read-only destinations,
cross-device staging, replaced parent paths, symlink/hardlink attacks, partial
backup loss, and process termination. Run on Linux and macOS hosted jobs.

**Acceptance:** Every injected failure retains or restores a matched working
pair; no path outside the pinned install directory changes; failure output
identifies manual recovery only when rollback genuinely cannot be verified.

### ND36-008 — Merge the v3.6.0 homepage version update

**Status:** Separate homepage PR prepared: [QubeTX/qube-machine-report-homepage#8](https://github.com/QubeTX/qube-machine-report-homepage/pull/8).

**Why needed/recommended:** Four homepage components carry a fallback ND300
version for GitHub/API failure cases. If the code release advances without the
homepage merge, install guidance can keep advertising 3.5.2 even though 3.6.0
is public.

**Impact:** Completing it keeps download/install instructions consistent. It
does not affect binary safety, so it must not delay an urgent security release.

**Acceptance:** The homepage PR is merged after the 3.6.0 release is public,
the production deployment succeeds, and all four fallback locations show
3.6.0.

## P1 — Mac-only follow-up when safe hardware is available

### ND36-009 — Qualify the service cycle on a disposable Mac service

**Status:** Production gate remains off. One user-authorized active-Mac run of
the exact ignored Rust acceptance path passed under an independent root
watchdog, and post-run Wi-Fi/DHCP/DNS/default-route state was healthy. That is
supplementary evidence only, not disposable-host qualification.

**Why needed/recommended:** The removed implementation could delete a network
service and incompletely recreate it; the user plausibly experienced lasting
Wi-Fi trouble after testing it. A simple happy-path run cannot prove recovery
from process death or command failure at every destructive transition, and an
active remote-control link is not a safe place to discover a rollback defect.

**Impact:** Completing the full matrix is the only acceptable basis for making
the high-risk Mac action available. Until then, users receive safe guidance
instead of automatic cycling. Leaving the gate off sacrifices one last-resort
repair action but eliminates the risk of shipping another destructive reset.

**Work:** Use a disposable VM or sacrificial physical service with local console
access and an independent offline/root watchdog. Run the checked-in
`scripts/run-macos-service-cycle-live.sh` harness only after reviewing its exact
interface/service acknowledgements. Inject failure and SIGINT/SIGTERM/SIGHUP at
snapshot, restore registration, disable, re-enable, DNS/search restoration,
route/reachability verification, and token resolution. Prove the launchd job
runs once (`KeepAlive=false`) and its durable terminal marker refuses rerun.

**Acceptance:** Every transition either leaves state unchanged or restores the
exact enabled/DHCP-or-static/DNS/search-domain/route state; restores drain LIFO;
watchdog evidence survives controller loss; no test can execute twice; only
then may a separate reviewed change consider flipping the production constant.

### ND36-010 — Repeat read-only real-Mac diagnostic smokes after final parsers

**Status:** Deferred when the user requested wrap-up.

**Why needed/recommended:** The last parser pass added `lsof`-based macOS
connection capture, wide IPv6/process fields, normalized DHCP lease wording,
stricter dormant-`utun` filtering, and bounded TLS/DNS evidence after earlier
real-host smokes. Fixtures and Rust tests cover these changes, but a physical
Wi-Fi Mac produces richer data than GitHub's headless runners.

**Impact:** Completing it provides confidence in presentation and live macOS 26
output. Deferring it does not expose users to mutation—the commands are
read-only—and hosted release jobs still verify architecture, version,
signature, quarantine, deployment floors, and Gatekeeper.

**Work:** On a future Mac, run native ARM table/core JSON/technician JSON plus
Rosetta x86_64 from quarantined public archives. Inspect, without publishing
private network values, interface count/name, connected Wi-Fi metadata,
listeners, IPv6 endpoints, DHCP normalization, VPN state, TLS evidence, and
DNSSEC wording. Do not run `fix`.

**Acceptance:** Output matches actual host state, both architectures terminate
within caps, redacted SSIDs remain unavailable, dormant tunnels are not VPNs,
and Gatekeeper reports `Notarized Developer ID` for both binaries.

## Completed in this branch

- Removed all macOS service deletion/recreation, Wi-Fi password retrieval, and
  SSID-scanning repair paths; added verified, registered-before-mutation restore
  behavior with a production-off gate.
- Preserved static/Manual/BootP network configuration by proving DHCP mode
  before renewal; exact DNS servers and search domains are restored and
  verified.
- Added LIFO signal-safe restoration, pre-disconnect consumer VPN restore
  registration, enterprise-VPN refusal, invoking-user validation, and atomic
  mode-`0600` report creation.
- Replaced Unix `curl|sh`/`wget|sh` updating with bounded exact-tag archive and
  SHA-256 verification, allowlisted extraction, exact-version checks,
  macOS trust checks, paired atomic replacement, rollback, and origin-aware
  uninstall.
- Hardened macOS 26 interface, Wi-Fi, route, listener, IPv6, DHCP, protocol,
  VPN, TLS/DNSSEC, timeout, and Apple networkQuality behavior with fixtures and
  fault tests; Windows virtual/default-adapter fixture coverage was extended.
- Automated fail-closed dual-architecture Apple signing/notarization and public
  Gatekeeper verification, plus final artifact and Windows-asset attestations.
- Recorded the destructive-repair, transactional-update/release-trust, and
  invoking-user decisions in `docs/architecture-decisions.md`; updated both
  changelogs and the project/agent/user documentation for v3.6.0.
