# ND300 Architecture Decisions

## ADR-0001: macOS repair must never delete a network service

- **Date:** 2026-07-17
- **Status:** Accepted for v3.6.0

### Context

The former macOS deep-reset implementation removed and recreated the selected
network service. Its snapshot covered only part of the service configuration,
some snapshot failures became empty defaults, restore command failures were
suppressed, and the recovery token could be resolved before restoration was
proved. A real-host report of persistent Wi-Fi trouble after exercising that
path corroborated the destructive failure mode.

macOS network services can carry configuration that ND300 cannot safely clone:
managed 802.1X state, DHCP client identity, proxy bypass/PAC settings, custom
routes, service enablement, and policy-managed configuration. Reconstructing a
partial copy is therefore not a safe recovery primitive.

### Decision

- ND300 will not call `networksetup -removenetworkservice` or
  `-createnetworkservice` during repair.
- The existing `DeepStackReset` action ID is retained for planner and report
  compatibility, but its macOS implementation is a high-risk, explicitly
  confirmed cycle of an existing physical network service.
- Device-to-service mapping comes from
  `networksetup -listnetworkserviceorder`; hardware-port labels are never used
  as service names. Ambiguous, disabled, virtual, VPN, or missing services are
  refused without mutation.
- A restore operation is registered before disabling the service. Re-enable is
  verified before the token is resolved, and unresolved restores drain LIFO on
  normal exit, interruption, signal, timeout, or caught panic.
- DHCP renewal is allowed only for a service already configured for DHCP.
  Manual, static, BootP, or indeterminate modes are left untouched.
- DNS rollback captures and verifies both DNS servers and search domains.
- The implementation is enabled only after state-machine/failure-injection
  tests and a disposable macOS environment prove the service-cycle path. The
  production gate remains false without that disposable-host evidence.
- A user-authorized active-Mac acceptance may exercise the exact private path
  only under an independent root watchdog that owns recovery before mutation.
  It is supplementary evidence, not permission to flip the production gate.
  The harness uses explicit `RunAtLoad=true`, `KeepAlive=false` launchd plists
  plus a durable terminal marker so a completed destructive test cannot run
  twice.

### Implementation and validation evidence

The production source contains no service delete/recreate command, and
regression tests forbid restoring the old password/SSID/snapshot path. Pure
fixtures and injected command outcomes cover renamed, disabled, ambiguous,
virtual, static, DHCP, DNS/search, signal, and LIFO-restore behavior.

On 2026-07-17, the user authorized one execution of the exact ignored Rust
acceptance path on the active Mac only after an independent root launchd
watchdog owned recovery. The path completed successfully and independent
readback found the service enabled with the expected DHCP address/router,
automatic DNS/search state, and default route. The exercise also revealed that
the first launchd submission form could restart a completed job. The harness
was changed to an explicit `RunAtLoad=true`, `KeepAlive=false` plist, durable
terminal-state marker, and tests that refuse a second execution.

This evidence answers whether the happy path can complete and whether the
watchdog can restore the observed configuration. It does not answer every
failure-transition question on a disposable service. Consequently the
production constant remains false; availability requires the separate matrix
recorded in `TASKS.md`.

### Consequences

The macOS last-resort action is less invasive than the Windows/Linux platform
equivalents, but it cannot destroy configuration ND300 does not fully
understand. Some failures will now end with explicit manual guidance instead of
an attempted automatic reset. That is the intended fail-safe behavior.

## ADR-0002: Unix updates are verified transactions; macOS releases are trusted artifacts

- **Date:** 2026-07-17
- **Status:** Accepted for v3.6.0

### Context

The Unix updater previously ran `rustup update stable`, piped a mutable
`releases/latest` installer into a shell, and could remove an older install
before proving the intended Cargo binary. The public v3.5.2 macOS archives were
also ad-hoc signed or unsigned and failed Gatekeeper assessment.

### Decision

- Updating ND300 must not update the user's Rust toolchain.
- Cargo updates verify both binaries at the explicit invoking user's Cargo bin
  directory before any legacy cleanup.
- The Unix archive fallback downloads an exact tag/target asset, verifies its
  SHA-256 digest in Rust, safely extracts only `nd300` and `speedqx`, validates
  their versions, and atomically installs both with rollback.
- Unknown and package-manager-owned install locations are never recursively
  deleted. Cargo installs use `cargo uninstall nd300`; a launcher symlink is
  removed as a symlink rather than by deleting its canonical target.
- macOS fallback binaries must pass Developer ID signature, identifier, Team ID
  (`M9D5379H93`), hardened-runtime, timestamp, and Gatekeeper install-policy
  acceptance with `source=Notarized Developer ID` before installation.
- Release CI signs both binaries in both thin Mac archives using an ephemeral
  keychain, requires Apple notarization status `Accepted`, preserves those exact
  bytes when repacking, and has no unsigned fallback. It then re-downloads both
  public Mac archives on native arm64/Intel runners, applies quarantine, and
  repeats signature, Gatekeeper, version, architecture, and deployment-floor
  checks. Attestations bind the final post-notarization artifacts.
- A release tag must be an exact stable version on the tested default-branch
  SHA, match `Cargo.toml`, and correspond to an already-published crates.io
  version before Apple secrets are exposed. Hosting requires every build job,
  exactly 22 base assets, two public Mac trust checks, then exactly 28 assets
  and exact-source attestations after the Windows add-ons.

The product is a standalone Mach-O CLI, not an `.app` or `.pkg`; it therefore
has no staplable ticket container. `notarytool` acceptance binds Apple's online
record to the signed code hash. For bare CLIs, `spctl --assess --type install
--ignore-cache --no-cache` reliably distinguishes `Notarized Developer ID`
from `Unnotarized Developer ID`; `spctl --type execute` and
`codesign --check-notarization` do not, and are never substitutes.

### Consequences

Unix update failures retain the working installation and return actionable
manual guidance. Releases cannot proceed when Apple credentials, signing,
notarization, or final trust verification are unavailable. Windows keeps its
existing four installer-aware update strategies.

GitHub repository credentials for the Apple workflow were provisioned during
the v3.6.0 hardening pass and authenticated without recording their values in
the repository. A Windows operator can therefore trigger the complete Mac
sign/notarize/verify flow after exact-SHA CI; continued access to the audit Mac
or its downloaded API key is neither required nor permitted.

## ADR-0003: Elevated Unix actions operate in the invoking user's context

- **Date:** 2026-07-17
- **Status:** Accepted for v3.6.0

### Decision

When effective UID is root under `sudo`, ND300 resolves numeric `SUDO_UID` and
`SUDO_GID` through the system passwd database. User reports, Cargo paths,
receipts, and user-scoped subprocesses use that validated identity rather than
the process's root `HOME`. Sensitive reports are created atomically with mode
`0600` and assigned to the invoking user.

Environment-provided usernames alone are not trusted, and a failed identity
lookup does not fall back to mutating an arbitrary user path.
