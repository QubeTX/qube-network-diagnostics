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
- [x] Final version, release/main SHAs, and run URLs are recorded
- [x] Crates.io, Apple trust, assets, checksums, and attestations are recorded
- [x] Windows matrix, Alienware, and homepage results are recorded
- [x] Technical and human changelogs remain aligned
- [ ] Closeout PR CI passes and merges without unrelated files

## Status
Active. Exact-source, public artifact, Apple, disposable matrix, successful Alienware Global MSI update, homepage, ADR, and changelog evidence are recorded on the dedicated v3.6.4 closeout branch. Only closeout-PR CI and merge remain.

## Activity
- 2026-07-17 10:18 — imported ND36-005 and converted it to a separate evidence PR (agent: codex)
- 2026-07-17 22:39 — created the v3.6.4 qualification record, linked the accepted updater ADR, and aligned the technical/human changelogs and task board; Alienware elevation is the only evidence field still pending (agent: codex)
- 2026-07-18 02:22 — replaced the pending physical section with the successful 3.5.2-to-3.6.4 Global MSI transaction, exact public hashes, fresh-shell ownership, cleanup, renderer/redaction, and unchanged-network evidence (agent: codex)
