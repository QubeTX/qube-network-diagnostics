TT;DR: After public release, prove `nd300 update` from every supported Windows origin and upgrade the Alienware's existing Global MSI copy from 3.5.2.

## Why
Builds prove that artifacts exist, not that v3.5.2 selects and installs the correct public 3.6.x artifact. Origin detection, paired binaries, fresh-shell PATH, marker lifecycle, and uninstall must work in real updates.

## Plan
Run the five-origin matrix on fresh GitHub-hosted Windows VMs using public artifacts and `nd300 update`; inspect marker, ARP, PATH scope, binary locations, versions, sidecars, migration dry-run, and uninstall. Then run the same updater from the Alienware's current 3.5.2 `msi-global` installation and verify both commands resolve to Program Files in a fresh shell. Do not use the physical machine for cross-edition/uninstall tests.

## Impact
This is the final Windows user-journey gate. Failure after tagging triggers the documented patch-bump loop rather than retagging.

## Acceptance
Every supported origin reaches one matched final-version binary pair at the intended location, incorrect/unknown ownership is refused, sidecars validate, uninstall removes only the proven origin, and the Alienware upgrade succeeds without network-state mutation.

## Verification
- [ ] Global MSI public updater case passes
- [ ] Corporate MSI public updater case passes
- [ ] Global EXE public updater case passes
- [ ] Corporate EXE public updater case passes
- [ ] Cargo public updater and stale-marker case pass
- [ ] Alienware fresh shell resolves both binaries to the final Global MSI version

## Status
Blocked on #rel.

## Activity
- 2026-07-17 10:18 — imported post-release portion of ND36-003 (agent: codex)
