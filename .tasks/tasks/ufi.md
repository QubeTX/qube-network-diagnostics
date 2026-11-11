TT;DR: Expand Unix updater transaction fault tests across crashes, filesystem boundaries, permission changes, and rollback transitions.

## Why
The updater already pins directories, uses no-follow operations, validates an exact two-binary pair, and rolls back atomically. More hostile process/filesystem testing would increase assurance against rare interruption and race shapes.

## Plan
Add deterministic fault points before/after backup, each rename, validation, rollback, and cleanup. Cover read-only targets, cross-device staging, replaced parents, symlink/hardlink attacks, partial backup loss, and process termination on hosted Linux and macOS.

## Impact
Reduces the chance of a mismatched or unrecoverable installed pair. Existing fail-closed behavior means this is assurance work, not a known production fix.

## Acceptance
Every injected failure preserves or restores a matched pair, no outside path changes, and manual-recovery output appears only when rollback truly cannot be verified.

## Verification
- [ ] Fault points cover every transaction transition
- [ ] Linux and macOS hosted matrices pass
- [ ] Pair integrity and path confinement invariants hold
- [ ] Manual-recovery reporting is evidence-based

## Status
Deferred P1; core Rust fault tests already exist.

## Activity
- 2026-07-17 10:18 — migrated ND36-007 to Backlog (agent: codex)
