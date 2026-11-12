TT;DR: Skip expensive CI and installer qualification for proven documentation-only changes, while preserving complete code and release validation.

## Why
The operator explicitly requested this on 2026-11-11 after the release-efficiency work. Documentation receipts currently rerun native builds, signing and installer lifecycles even though the product is unchanged. Whole-workflow path filters can leave required checks pending; an always-running classification and required-check path must distinguish intentional skips from missing or failed validation.

## Scope
Use a narrow regular-file Markdown allowlist for ordinary PR and main pushes. Inspect the entire PR or push, including both sides of renames; unknown paths, missing history, mixed changes, executable/symlink changes, release inputs, tags and explicit dispatches take the full path. Preserve the existing applicable Windows matrix path coverage. Keep exact-source full-CI and nine-job Apple qualification proof for releases; docs-only success must never authorize crate publication or a tag release. No product version, published artifact, credential or protection-rule changes.

## Plan
Implement shared classification, strict required-check aggregation, conditional expensive jobs, and exact-run CI evidence verification before publishing. Exercise boundary fixtures locally and full hosted CI for this workflow change. Then use a real documentation-only evidence PR and its main merge to prove the fast path, successful required checks and no-op publisher without a new product release.

## Impact
Routine documentation changes should finish with lightweight checks instead of native compilation and installer testing. Incorrect classification could bypass validation, so a positive allowlist and complete Git diffs are mandatory. Full CI markers and exact-run evidence prevent a documentation-only green result from being treated as release qualification. Tagging a documentation-only successor SHA is refused until that exact source has full validation; normal version changes already trigger it.

## Acceptance
Functional: ordinary docs-only PR/main changes skip expensive Rust/parity/native installer jobs and keep CI required successful; actual code/release changes retain all applicable checks. Evidence: real Git boundary fixtures, workflow contract tests, independent security review, full exact-head/main validation for implementation, then observed docs-only PR/main results. Stop after these bounded proofs; no unrelated build-system redesign or product release.

## Verification
- [x] Classifier fixtures cover complete PR/multi-commit push diffs, mixed/unknown paths, renames, mode/type changes and malformed or missing metadata
- [x] Required-check and publication fixtures reject accidental skips, failed/missing/mismatched evidence and docs-only release qualification
- [ ] Independent review, workflow lint and exact-source full hosted implementation checks pass
- [ ] Real documentation-only PR/main runs skip expensive jobs, keep CI required green, and leave publication disabled; receipts recorded

## Status
Implementation PR #51 merged as dbc784b3b51702928603d8041f045a78bbd6a0fc after final head 9566695971db61e09723761d88a53e341e430b72 passed all five hosted workflows, including both required/full CI markers and the complete Windows matrix. All 65 local fixtures, workflow/script lint and independent reviews passed. Exact-main CI 43153071950, Mac qualification 43153071864 and parity 43153071927 are being verified. This documentation-only evidence change begins the real fast-path proof; PR/main skips and publisher no-op remain open until observed. No product/version changes.

## Activity
- 2026-11-11 — full hosted CI 43150407308 passed its required checks but exposed implicit success propagation skipping the Full CI marker; corrected the condition to require successful classification and required gates with an explicit cancellation-aware status function; fresh hosted validation remains required before merge (agent: codex)
- 2026-11-11 — operator authorized documentation-only CI optimization and delivery; established narrow allowlist, complete-diff and exact-release-proof requirements (agent: codex)
- 2026-11-11 — implemented shared classification, strict required/full markers, native/parity gates and exact-run publication evidence; independent review and 65 focused fixtures passed, including explicit shallow-history rejection; hosted gates remain open (agent: codex)
- 2026-11-11 — merged #51 after exact-head CI 43151456264, Windows 43151456322, Mac 43151456336, parity 43151456602 and plan 43151456423 passed; the job-marker interpreter recognized the real PR jobs as full CI, separately from main-only publication authorization; recording implementation receipts in a Markdown-only PR while exact-main qualification runs (agent: codex)
