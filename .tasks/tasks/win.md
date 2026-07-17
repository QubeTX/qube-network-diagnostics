TT;DR: Make Windows update/uninstall ownership provable so a stale shared marker cannot route Cargo or portable copies through the wrong installer. Preserve the installer-internal no-escalation contract.

## Why
Static review found the four installer paths, marker values, identifiers, and packaged binaries in lockstep, but top-level `nd300 uninstall` only deletes files. It can leave ARP, system PATH, and `HKCU\\Software\\ND300\\InstallSource`, causing a surviving Cargo copy to reinstall an unrelated MSI/EXE on its next update.

## Plan
Create a Windows install-owner resolver that correlates the running executable with known HKCU/HKLM uninstall records, normalized install location, scope, format, and marker. A proven registered owner wins; `.cargo\\bin` overrides a contradictory marker; unknown locations never inherit installer authority. Top-level uninstall delegates to a proven MSI/Inno uninstaller without shell expansion and otherwise retains safe allowlisted deletion. Add a hidden validated `--install-origin` value to installer-internal migration and pass it from all four installers so custom locations identify their current edition. Migration never clears the newly installed marker and reports retained registration/PATH honestly.

## Impact
Intended: uninstall removes the actual owning product and later updates choose the surviving origin correctly. Risks: invoking the wrong registered command could remove unrelated software; matching must therefore be exact and fail closed. Nested cross-edition MSI unregistration remains out of scope because migration runs inside an active transaction.

## Acceptance
Unit tests cover marker/path/registered-owner precedence, stale marker plus Cargo, custom installer paths, safe command construction, unknown refusal, and migration marker preservation. Disposable Windows tests confirm ARP/PATH/marker lifecycle for the four installers plus Cargo.

## Verification
- [x] Windows origin-resolution unit tests pass on Rust 1.97.0
- [ ] Top-level MSI and Inno uninstall integration tests remove ARP, correct PATH hive, marker, and both binaries
- [ ] Cargo survives installer uninstall and remains classified as Cargo despite a stale-marker fixture
- [x] All four installers pass explicit origin values and the LOCKSTEP reviewer reports no drift
- [ ] `migrate-cleanup --dry-run --json` remains advisory and fail-closed

## Status
Implementation is complete and passes local Windows compile/Clippy plus origin-precedence tests. A permanent candidate/public five-origin workflow and evidence script are added; its fresh-VM results remain required before merge.

## Activity
- 2026-07-17 10:18 — created from installer lifecycle audit and operator-approved hardening plan (agent: codex)
- 2026-07-17 11:42 — implemented registered-owner resolution, Cargo stale-marker override, direct validated MSI/Inno uninstall launch, hidden installer origins, and file-only migration (agent: codex)
- 2026-07-17 11:42 — added candidate/public disposable Windows matrix for four installers plus Cargo, including ARP/PATH/marker snapshots, dry-run migration, uninstall, and stale-marker survival (agent: codex)
- 2026-07-17 12:19 — installer audit found and closed two pre-push defects: candidate artifacts are staged flat before matrix download, and MSI uninstall resolves the absolute protected System32 executable via `GetSystemDirectoryW` before elevation (agent: codex)
- 2026-07-17 12:20 — full 306-library/5-main Windows tests and all-target/all-feature Clippy with `-D warnings` passed after the security fix (agent: codex)
- 2026-07-17 12:31 — final installer LOCKSTEP/provenance audit reported no remaining supported-state findings across all four packages, updater/uninstall/migration, exact-SHA release flow, and five-origin matrix (agent: installer-lockstep-reviewer)
- 2026-07-17 12:51 — first hosted matrix run proved the flattened candidate artifact correct but exposed two harness defects before product lifecycle assertions: GUI-subsystem MSI/Inno processes could leave stale `$LASTEXITCODE`, and strict mode rejected unrelated ARP keys without `DisplayName`; added explicit process waiting, bounded binary materialization, and safe optional registry-property access (agent: codex)
- 2026-07-17 13:08 — targeted installer review traced the patched harness through all five lifecycle phases and reported it safe to push with no findings; parser, explicit process, argument-preservation, strict-mode registry, and sequencing checks all passed (agent: installer-lockstep-reviewer)
