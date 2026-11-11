TT;DR: Verify production publication through the environment-backed Apple signing pipeline on the next normally authorized release. This does not authorize a new release by itself.

## Why
The operator-approved security remediation preserves the current release format, but signed candidate qualification cannot prove a future public upload, attestation, or legacy bridge. Keep that distinct final evidence visible.

## Scope
The first normal tag release after #asign. Preserve immutable tags/assets and the 32-asset inventory. No early release or version bump solely for this task.

## Acceptance
Exact tag SHA passes post-merge qualification, all release targets, public archives, both installer workflows, all 32 assets/attestations, and both v3.7.2 compatibility bridges. Existing release and installer skills own the gates.

## Verification
- [ ] Exact tag run passes archive signing and both installer workflows without repository Apple secrets
- [ ] Exactly 32 public assets verify checksums and exact-source attestations
- [ ] Native Intel and Apple Silicon compatibility bridges and public Apple trust checks pass

## Status
Backlog awaiting the next user-authorized release. #asign is complete: environment-only signing and native lifecycles passed at 0db6d767b42ef10fd4300e21ee57cdf6e489eaae (qualification 37558357108; CI 37558357057). This candidate evidence does not replace the publication checks above.

## Activity
- 2026-10-06 — #asign completed and the critical finding was closed after credential isolation, native qualification and cleanup. Preserve this task for the next normal tag; no new release was created for remediation (agent: codex)
- 2026-10-06 — recorded the explicitly deferred production-publication proof from the accepted plan (agent: codex)
