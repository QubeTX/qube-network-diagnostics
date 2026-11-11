TT;DR: Shorten release work by reusing proven Windows binaries and cached audit tooling, while preserving required validation.

## Why
The operator asked whether CI/CD is needlessly slow, then explicitly requested the justified improvements on 2026-11-11. Measured release run 42972625036 spent 246 seconds recompiling Windows binaries for add-on installers after the same release already built them. CI cargo-audit installation costs 153–195 seconds; actual auditing takes seconds. PR48 CI took 4m43s, the separate native Mac build 6m05s, and Windows Cargo lifecycle adds a longer source-install tail. Repeated release preparation also contributed to session duration and must not be attributed solely to CI.

## Scope
Reuse the exact tag/source Windows ZIP with checksum and provenance verification for release installer add-ons; cache a pinned cargo-audit executable using exact host/toolchain/version identity, keeping advisory refresh and auditing. Preserve all installer lifecycle, 32-asset, source attestation, protected-signing and no-overwrite gates. Do not change product version or existing published artifacts. Broader cross-workflow artifact sharing, docs-only filters, Cargo lifecycle caching and publisher gate deduplication are deferred because they require separate design/measurement.

## Plan
Implement the two narrow changes, add focused acquisition failure checks, exercise real release archive acquisition and installer building without publication, inspect audit cache behavior, independently review, then merge through required CI. Use separate worktree from active release recovery.

## Impact
Expect about four minutes off the tag packaging critical path, plus roughly three runner-minutes per warm audit run. Wall-clock audit savings depend on whether it is the longest job. Do not promise measured future tag speed from a single dry run.

## Acceptance
Functional: packaging consumes only a verified exact-source Windows pair with no compile fallback; unchanged installer outputs build; cached audit version is checked and current advisories still run. Evidence: focused tests, Actionlint, independent security/installer review, hosted nonpublishing smoke and exact-head CI. Main delivery is workflow-only, no new product release needed. If the artifact source cannot be proven, fail closed and keep the task open.

## Verification
- [ ] Negative acquisition fixtures reject bad source, checksum, archive shape or version; valid pair builds all required add-on installers
- [ ] Hosted nonpublishing acquisition/package smoke passes against an existing immutable release
- [ ] Audit cache path and uncached installation both retain version validation and fresh audit execution
- [ ] Independent review and exact-head required CI pass; merged main behavior and receipts recorded

## Status
Active implementation in isolated codex/ci-release-efficiency. Original release recovery continues separately. No checks have been removed and no public artifact changes are authorized by this optimization.

## Activity
- 2026-11-11 — operator authorized measured CI/CD simplification; selected verified Windows binary reuse and pinned audit-tool caching, preserving release validation (agent: codex)
