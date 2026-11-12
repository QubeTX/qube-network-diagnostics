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
Implemented in codex/docs-only-ci at baseline c6044c193c3c1055741e7292cb6be2960dacb4bd. All 65 Python fixtures, all-workflow Actionlint and independent security/CI review passed. The verifier also refused a real pre-marker CI run instead of mistaking its aggregate success for full proof. Next: full hosted implementation PR/main checks, then a real docs-only PR/main proving skipped expensive jobs, green required checks and publisher no-op. No product/version changes.

## Activity
- 2026-11-11 — operator authorized documentation-only CI optimization and delivery; established narrow allowlist, complete-diff and exact-release-proof requirements (agent: codex)
- 2026-11-11 — implemented shared classification, strict required/full markers, native/parity gates and exact-run publication evidence; independent review and 65 focused fixtures passed, including explicit shallow-history rejection; hosted gates remain open (agent: codex)
