TT;DR: Merge PR #18 only after its exact final head is fully green, then prove the resulting main SHA and crates.io 3.6.0 publication before tagging.

## Why
ND-300 publishes the crate from a successful main push, separately from tag-triggered artifacts. Merging a merely older-green SHA or tagging before crates.io completes would violate the release gate and can produce mismatched public channels.

## Plan
Review the final diff and specialist audits; update PR body/evidence; merge with a merge commit so the reviewed head remains in history; record the merge SHA; monitor exact-main CI and `crates-publish.yml`; verify crates.io reports 3.6.0. Do not force, amend pushed commits, or bypass hooks/checks.

## Impact
This makes 3.6.0 public on crates.io and creates the only SHA eligible for the release tag. A failed main CI or crate publish blocks tagging.

## Acceptance
PR #18 is merged normally; the reviewed head is an ancestor of the merge SHA; exact-main CI succeeds; crates.io serves 3.6.0; no tag exists before those facts are recorded.

## Verification
- [x] Final PR head checks and both specialist reviews pass
- [x] Merge commit contains the reviewed PR head
- [x] Exact-main CI run succeeds
- [x] crates-publish run succeeds or idempotently confirms 3.6.0
- [x] crates.io API returns nd300 3.6.0 before tagging

## Status
Done. PR #18 reviewed head `4b2c623` was preserved by merge commit `2dd0f92`; exact-main CI `29602623257` and crates publish `29603068548` succeeded, and crates.io served unyanked 3.6.0 before the tag was created.

## Activity
- 2026-07-17 10:18 — imported ND36-001 and made exact-main crate publication an explicit gate (agent: codex)
- 2026-07-17 13:07 — merged PR #18 with merge commit `2dd0f92`, preserving reviewed head `4b2c623` (agent: codex)
- 2026-07-17 13:16 — exact-main CI `29602623257`, crates publish `29603068548`, and public crates.io 3.6.0 all verified before tagging (agent: codex)
