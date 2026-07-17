TT;DR: Push one immutable final version tag and verify the complete signed, notarized, checksumed, and attested 28-asset release.

## Why
v3.5.2 Mac artifacts failed normal Gatekeeper trust. v3.6.0 moves both-architecture signing/notarization, public quarantine assessment, exact-byte checksums, and provenance into GitHub Actions; bypassing any of those recreates the defect this release fixes.

## Plan
Create the unused tag at the exact green main SHA and push once. Monitor Release, reusable Windows add-ons, public Mac verification, and announcement. Verify 22 base assets followed by exactly 28 final assets, sidecars, aggregate checksum, source-digest attestations, Apple `Accepted`, Gatekeeper, versions, and installer aliases. Rerun transient failures; use a new patch version for genuine defects.

## Impact
This deploys binaries/installers to GitHub and advances the latest public release. A genuine post-tag defect requires fix-forward; tags and published assets are never moved or deleted without separate operator authorization.

## Acceptance
All release jobs succeed; both Mac archives are Developer ID signed, Apple accepted, and Gatekeeper approved; asset inventory is exactly 28; every checksum and attestation matches the tag SHA; all public binaries report the final version.

## Verification
- [ ] Version tag resolves to the exact green main SHA
- [ ] Release and reusable Windows installer jobs succeed
- [ ] Both Apple submissions report Accepted and public Gatekeeper checks pass
- [ ] Exact 28-asset inventory and all checksums pass
- [ ] Every artifact attestation verifies against the tag source digest
- [ ] Public binaries/installers report the final release version

## Status
Blocked on #mrg. Candidate version is 3.6.0; genuine release failures move forward to a patch version.

## Activity
- 2026-07-17 10:18 — imported ND36-004 and incorporated the exact-source workflow correction (agent: codex)
