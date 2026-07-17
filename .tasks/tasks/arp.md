TT;DR: Design a future cross-edition cleanup that removes stale Windows installer registration only after the active MSI/Inno transaction has safely completed.

## Why
Installer-internal `migrate-cleanup` deliberately performs allowlisted file cleanup inside installation. Invoking another MSI uninstaller there would nest Windows Installer transactions and is unsafe, but file-only cleanup can retain an inactive ARP entry or system PATH record.

## Plan
Evaluate an after-finalize, reboot-safe, idempotent cleanup handoff that proves the old owner, runs the owning uninstaller outside the active transaction, restores/verifies the new shared marker, and never escalates silently. Keep v3.6 migration reporting honest until this has independent installer/rollback tests.

## Impact
Would make cross-edition consolidation clean at registration level, not only executable-resolution level. A flawed implementation could unregister the new edition or trigger installer recursion, so this remains non-release-blocking.

## Acceptance
The design cannot run inside an active installer transaction, proves old/new ownership, preserves the new marker, handles user/system scope, is idempotent, and passes four-direction upgrade/rollback tests.

## Verification
- [ ] Transaction-boundary design is documented and reviewed
- [ ] Old and new ownership are independently proven
- [ ] Four cross-edition directions pass clean install/rollback tests
- [ ] No nested MSI transaction or silent elevation occurs

## Status
Deferred P1 from the Windows installer audit. v3.6 release acceptance requires honest residual reporting and one active binary pair, not nested unregistration.

## Activity
- 2026-07-17 10:18 — created to preserve the accepted non-release-blocking installer boundary (agent: codex)
