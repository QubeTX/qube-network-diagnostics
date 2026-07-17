TT;DR: Replace release-candidate assumptions with exact public evidence in a small post-release PR while preserving the rationale behind every gate.

## Why
Future maintainers need to distinguish local tests, hosted exact-SHA CI, crates.io publication, Apple acceptance, public artifacts, Windows upgrade matrices, and physical Alienware evidence. Premature or missing closeout makes incident response unreliable.

## Plan
Create a dedicated closeout branch after release. Update technical and human changelogs, project documentation, and this board with final version/SHA, run IDs/URLs, crate result, Apple status, 28-asset/checksum/attestation result, Windows matrices, Alienware smoke/update, homepage deployment, and any waived P1 items. Merge through normal CI; unchanged crate version should make publish idempotently skip.

## Impact
The repository becomes an evidence-backed source of truth without altering published binaries. Risks are documentation drift or accidentally bundling unrelated changes, so stage specific files only.

## Acceptance
No completed claim lacks evidence, no completed gate remains described as pending, both changelogs stay in lockstep, task verification items are closed or explicitly waived, and the closeout PR is green and merged.

## Verification
- [ ] Final version, release/main SHAs, and run URLs are recorded
- [ ] Crates.io, Apple trust, assets, checksums, and attestations are recorded
- [ ] Windows matrix, Alienware, and homepage results are recorded
- [ ] Technical and human changelogs remain aligned
- [ ] Closeout PR CI passes and merges without unrelated files

## Status
Blocked on #web.

## Activity
- 2026-07-17 10:18 — imported ND36-005 and converted it to a separate evidence PR (agent: codex)
