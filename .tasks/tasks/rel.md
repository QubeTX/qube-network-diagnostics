TT;DR: Push one immutable final version tag and verify the complete signed, notarized, checksumed, and attested 28-asset release.

## Why
v3.5.2 Mac artifacts failed normal Gatekeeper trust. The v3.6 line moves both-architecture signing/notarization, public quarantine assessment, exact-byte checksums, and provenance into GitHub Actions; bypassing any of those recreates the defect this release fixes. The immutable v3.6.0 attempt exposed a release-validator defect, so v3.6.1 is the required forward-only final release.

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
Active. v3.6.0 correctly remains at `2dd0f92`; its Intel metadata-parser failure was closed in v3.6.1. Tag v3.6.1 correctly remains at `d8e246e`; run `29606285691` passed both Apple/public trust lanes and published all 28 valid assets, but announcement stopped because the checksum verifier rejected cargo-dist's whitespace-only separator. Fix forward to v3.6.2; never move or reuse either prior tag.

## Activity
- 2026-07-17 10:18 — imported ND36-004 and incorporated the exact-source workflow correction (agent: codex)
- 2026-07-17 13:19 — pushed immutable v3.6.0 at exact green main `2dd0f92`; release run `29603383832` started (agent: codex)
- 2026-07-17 13:26 — run stopped after Intel Apple `Accepted` and Gatekeeper success because valid legacy `LC_VERSION_MIN_MACOSX/version 10.12` output was rejected by a modern-only parser; v3.6.1 fix-forward opened (agent: codex)
- 2026-07-17 13:36 — v3.6.1 shared metadata parser and positive/negative fixtures passed LF-exact ShellCheck, actionlint, strict Clippy, 312 tests, release build, audit, 102-file package, publish dry-run, and all four local installer builds with exit 0 (agent: codex)
- 2026-07-17 13:58 — PR #19 exact head `a908e00` passed CI `29604943619` and five-origin candidate matrix `29604943583`; merge `d8e246e` passed main CI `29605809330` and crates publish `29606164285` (agent: codex)
- 2026-07-17 14:19 — v3.6.1 run `29606285691` passed Intel/arm64 notarization and public Gatekeeper, hosted exactly 28 assets, then rejected a trailing blank `sha256.sum` line before announcement (agent: codex)
- 2026-07-17 14:22 — downloaded the public v3.6.1 set and independently verified 11 sidecars, eight nonblank aggregate entries, and all 28 attestations against source `d8e246e` and the repository signer; v3.6.2 shared verifier/fixtures started (agent: codex)
- 2026-07-17 14:32 — v3.6.2 shared verifier passed blank-separator, malformed-entry, empty-manifest, corruption, and public-v3.6.1 replay fixtures; actionlint, strict Clippy, 312 Rust tests, release build, audit, 102-file package, publish dry-run, and all four installer builds passed (agent: codex)
