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
recorded in `.tasks/TASKS.md`.

### Consequences

The macOS last-resort action is less invasive than the Windows/Linux platform
equivalents, but it cannot destroy configuration ND300 does not fully
understand. Some failures will now end with explicit manual guidance instead of
an attempted automatic reset. That is the intended fail-safe behavior.

## ADR-0002: Self-update preserves the proven channel and uses immutable transactions

- **Date:** 2026-07-18
- **Status:** Accepted; extended for the v3.7.3 direct-package transition

### Context

ND300 can be installed through Cargo, a managed macOS/Linux archive, the native
macOS Apple Installer package, a Windows standalone archive, or one of four
Windows installers (Global/Corporate × MSI/EXE). Treating those routes as
interchangeable changes ownership, privilege, PATH, registration, and uninstall
behavior. Updating a running Windows image also cannot rely on overwriting the
executable in place. On macOS, a checksum alone does not establish that
downloaded code or an installer is trusted by Apple.

### Decision

- `nd300 update` and `speedqx update` first prove one owner, then execute only
  that owner's channel. Cargo stays Cargo; a validated Unix receipt stays on the
  managed archive path; a receipt-and-hash-proven macOS package downloads the
  exact tagged PKG and reopens Apple Installer; Windows standalone stays
  standalone; and each registered MSI/EXE edition downloads its exact matching
  installer. A marker or familiar path is supporting evidence, never authority
  over the running executable and a proven package-manager/installer record.
- Unknown, conflicted, package-manager-owned, and local-build locations fail
  closed without mutation. The error names the ownership problem and links to
  `https://reports.qubetx.com/nd300#install` for a fresh same-channel install.
- In JSON mode, stdout contains exactly one final response object. Download,
  checksum, installer, and child-process progress uses stderr so automation can
  parse success and safe-failure outcomes without stripping terminal text.
- Public entrypoints remain versionless (`releases/latest`, installer aliases,
  and `cargo install`). After resolving latest once, the transaction pins the
  exact stable version, tag, target asset, checksum sidecar, and source digest.
  Cargo uses `--version =<resolved> --locked`; an update must not update Rust or
  silently accept a later crate.
- `nd300` and `speedqx` are one package transaction. Downloads are verified and
  both candidate versions are checked before mutation. Unix stages the pair and
  receipt, replaces atomically, and rolls back both on failure. Windows retires
  only the two allowlisted running images to versioned siblings, uses MSI/Inno or
  the hardened standalone installer for replacement/rollback, then removes a
  non-running retired pair synchronously. A genuinely locked member uses the
  trusted delayed-delete helper and is reported as scheduled; an unschedulable
  sibling retains the primary command and reports incomplete cleanup.
- Update intent and fresh-install intent are distinct. An update always keeps
  the proven channel. A fresh official installer is the user's newest intent and
  may repair or downgrade that installer channel. On macOS, package postinstall
  may supersede only a receipt- or Cargo-registration-proven managed pair. On
  Windows, a fresh installer may replace a proven MSI/EXE format only within the
  same per-user or per-machine scope. Opposite-scope takeover is refused before
  mutation.
- Uninstall follows the same ownership proof. Registered Windows installs
  delegate directly to the validated MSI product or Inno uninstaller without
  shell-expanding registry commands. Cargo, managed archive, symlink, and
  portable cleanup remain origin-specific and narrowly allowlisted.
- macOS managed-archive binaries must pass Developer ID identity, per-binary
  identifier, Team ID `M9D5379H93`, hardened runtime, timestamp, and Gatekeeper
  install-policy acceptance with `source=Notarized Developer ID` before
  installation. The native macOS installer artifact is the versionless universal
  `nd300-universal-apple-darwin.pkg`, signed with Developer ID Installer,
  notarized, stapled, and accepted by Gatekeeper. The PKG owns exactly the universal
  `nd300`/`speedqx` pair plus a root-owned hash-bound install receipt, and its
  identifier, version, payload, signing identity, and trusted timestamp are
  verified before Apple Installer opens. The public JSON strategy
  `mac_dmg_pkg` and receipt channel `macos-dmg-pkg` remain stable compatibility
  identifiers; changing transport does not break installed receipts or consumers.
- Package update downloads the exact-tag PKG and checksum sidecar to private
  temporary storage, validates the Developer ID Installer identity, timestamp,
  stapled ticket, Gatekeeper install policy, exact payload, package metadata,
  and version, opens Apple Installer with a bounded wait, and then re-proves the receipt, hashes, and
  versions. Cancellation or failure leaves the installed pair in place and
  reports the exact matching tagged artifact plus the versionless install page.
  Package uninstall removes only the proven pair and receipt and forgets
  `com.qubetx.nd300.pkg` through an authorized native command.
- The signed/notarized/stapled DMG and its sidecar remain public solely because
  immutable v3.7.1 and v3.7.2 clients request that name. It contains the exact
  same PKG bytes as the preferred direct artifact. Native Intel and Apple
  Silicon validation proves that byte identity in addition to the direct-PKG
  install, repair, downgrade, update-selection, uninstall, and reinstall lifecycle.
- Release CI requires Apple notarization status `Accepted`, re-verifies the
  candidate on native Intel and ARM hosts, and later re-verifies the exact public
  quarantined bytes on both architectures. After publication, both native hosts
  install the immutable v3.7.2 package baseline and prove the new compatibility
  DMG contains the exact direct-PKG bytes and converges the receipt-bound pair
  without a duplicate. Completing the old GUI updater remains the final
  physical-Mac acceptance because hosted runners cannot approve Apple Installer prompts.
- A release tag is created once at the exact green default-branch SHA only after
  the matching crate is public. Release and reusable Windows workflows retain
  that tag context, publish exactly 32 assets, verify every sidecar and aggregate
  checksum, and bind every attestation to the tag SHA before announcement.

The managed archive remains supported for existing users and is a standalone
Mach-O archive, so it has no staplable ticket container. `notarytool` acceptance
binds Apple's online record to the signed code hash. For bare CLIs, `spctl
--assess --type install --ignore-cache --no-cache` distinguishes `Notarized
Developer ID` from `Unnotarized Developer ID`; `spctl --type execute` and
`codesign --check-notarization` are not substitutes. The native package adds
staplable containers without weakening the archive channel's ownership,
exact-version, paired-binary, rollback, or trust rules.

### Consequences

ND300 never changes channel merely because the intended strategy failed. A
failure retains or restores the last working pair and gives same-channel manual
recovery guidance. Already-published clients cannot be changed retroactively,
so release artifacts named by a safe older updater remain available while the
versionless entrypoints evolve. Documentation recommends the managed-archive
shell installer on macOS/Linux and the PowerShell standalone installer on
Windows; native packages remain explicit channel choices. Releases stop when ownership, checksum,
rollback, Apple trust, asset inventory, or provenance cannot be proved.

The first complete archive-channel qualification is documented in [ND-300
v3.6.4 release qualification](release-evidence-v3.6.4.md). The direct-package
transition receives its final exact-SHA, Apple, public-matrix, and physical-Mac
qualification record after the immutable v3.7.3 release completes.

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
