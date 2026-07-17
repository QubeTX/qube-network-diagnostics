TT;DR: Repair the concrete CI failures and release-provenance race on PR #18, then add the two missing Linux release-target compile gates. The task is active at the original red head `eee67f5`.

## Why
This is a direct operator order and the first release gate. The Mac proved both Apple architectures, but exact-head CI exposed target-only Rust warnings and ShellCheck diagnostics. Independent workflow review also proved that `workflow_run` can attest the latest default-branch SHA instead of the tagged source, so fixing only the visible lint errors would still leave release provenance unsafe.

## Plan
Apply narrow cfg/import/test-order fixes; use scoped ShellCheck suppressions for trap-only functions; convert the Windows add-on workflow to `workflow_call` plus safe tag-bound manual dispatch; invoke it from the tag-push Release workflow with validated tag/SHA and strict permissions; make announcement depend on its success; add permanent musl and Linux ARM compile jobs using Rust 1.97.0 and appropriate cross toolchains. Keep the production Mac service-cycle constant off.

## Impact
Intended: PR CI becomes meaningful across every published Rust target and every release artifact attestation binds to the immutable source. Possible risk: reusable-workflow expression or permission drift can prevent tag deployment, so actionlint, cargo-dist plan, exact-head CI, and release monitoring are mandatory.

## Acceptance
All five board subtasks complete; actionlint and ShellCheck pass; current Windows/Linux failures disappear without blanket allows; both extra Linux targets compile; reusable Windows add-on attestations retain exact tag-SHA verification; PR checks are green on the exact pushed head.

## Verification
- [x] `cargo +1.97.0 fmt --all -- --check` passes
- [ ] Windows and Linux Clippy/test/release-build CI jobs pass on the exact PR head
- [ ] ShellCheck release-scripts job passes
- [ ] Linux ARM GNU and x86_64 musl release compile gates pass
- [x] `actionlint` and cargo-dist plan pass with the reusable workflow
- [x] Exact-source and same-repository-signer attestation verification remains enabled

## Status
Implementation is complete locally. Rust 1.97.0 is selected; strict Windows Clippy, targeted tests, PowerShell parsing, `git diff --check`, and actionlint pass. Exact-head hosted CI, ShellCheck, both cross targets, and cargo-dist plan remain the acceptance evidence.

## Activity
- 2026-07-17 10:18 — moved to Active; imported CI logs and provenance finding from planning audits (agent: codex)
- 2026-07-17 10:18 — completed tracked board migration subtask (agent: codex)
- 2026-07-17 11:42 — completed cfg/import/parser-order and scoped ShellCheck fixes; converted Windows add-ons to an exact-SHA reusable release job; added Linux ARM GNU and x86_64 musl compile gates (agent: codex)
- 2026-07-17 11:42 — local Rust 1.97.0 actionlint, strict Clippy, targeted tests, and diff checks passed; hosted exact-head gates remain (agent: codex)
- 2026-07-17 12:21 — required cfg reviewer found no current-tree findings; final 28-asset verification now constrains both exact tag digest and repository signer (agent: codex)
- 2026-07-17 12:31 — required installer reviewer returned `LOCKSTEP OK`; exact pushed-head CI remains the only merge gate (agent: installer-lockstep-reviewer)
- 2026-07-17 12:38 — first exact-head Linux ARM gate exposed missing cross-libc headers because `--no-install-recommends` omitted them; added explicit `libc6-dev-arm64-cross` rather than weakening the new gate (agent: codex)
