TT;DR: Verify production publication through the environment-backed Apple signing pipeline on the next normally authorized release. This does not authorize a new release by itself.

## Why
The operator-approved security remediation preserves the current release format, but signed candidate qualification cannot prove a future public upload, attestation, or legacy bridge. Keep that distinct final evidence visible.

## Scope
The first normal tag release after #asign. Preserve immutable tags/assets and the 32-asset inventory. No early release or version bump solely for this task.

## Acceptance
Exact tag SHA passes post-merge qualification, all release targets, public archives, both installer workflows (with separately recorded delivery repair where needed), all 32 assets/attestations, and both v3.7.2 compatibility bridges. Existing release and installer skills own the gates.

## Verification
- [x] Exact tag/source archive and installer jobs pass without repository Apple secrets; failed root run and successful repair are separately recorded
- [x] Exactly 32 public assets verify checksums and exact-source attestations
- [x] Native Intel and Apple Silicon compatibility bridges and public Apple trust checks pass

## Status
Complete. Immutable4.0.2 source ceb10bf has all32 public assets, exact-source/tag attestations, public native archive trust, direct package lifecycles, byte-identical DMG package, and both compatibility bridges. Root42972625036 stays failed; its positive base jobs plus successful Mac repair42982227834 attempt3 satisfy the delivery evidence. Protected-main reusable credential preflight42981719549 also passed both native rows with all production rows skipped. No repository Apple secrets or changed protection rules. See docs/release-evidence-v4.0.2.md.

## Activity
- 2026-10-06 — #asign completed and the critical finding was closed after credential isolation, native qualification and cleanup. Preserve this task for the next normal tag; no new release was created for remediation (agent: codex)
- 2026-10-06 — recorded the explicitly deferred production-publication proof from the accepted plan (agent: codex)
- 2026-11-11 — activated for the normal authorized 4.0.2 release; retain all production publication and native compatibility gates until verified (agent: codex)
- 2026-11-11 — 4.0.2 exposed a reusable-workflow secret-interface defect absent from standalone qualification; retain the publication gate for forward 4.0.3 and record the incomplete 28-asset attempt (agent: codex)
- 2026-11-11 — completed environment-backed production proof through positive base jobs and successful exact-tag repair; independently verified32assets and source/ref attestations plus both native bridges. Kept original failure and two CloudKit transport retries in the evidence (agent: codex)
