TT;DR: Delivered verified Windows binary reuse and audit-tool caching through PR49. Hosted proof built all three add-on installers; the observed warm audit job fell from 204 to 18 seconds while still fetching advisories.

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
- [x] Negative acquisition fixtures reject bad source, checksum, archive shape or version; valid pair builds all required add-on installers
- [x] Hosted nonpublishing acquisition/package smoke passes against an existing immutable release
- [x] Audit cache path and uncached installation both retain version validation and fresh audit execution
- [x] Independent review and exact-head required CI pass; merged main behavior and receipts recorded

## Evidence
| Criterion | Oracle / invocation | Result / pointer | Interpretation | Limitation | Status |
|---|---|---|---|---|---|
| Exact-source acquisition and installer outputs | Focused PowerShell fixture suite plus hosted `validate_only` dispatch | [Run 42984456696](https://github.com/QubeTX/qube-network-diagnostics/actions/runs/42984456696), tooling ecab180, product ceb10bf/v4.0.2; downloaded receipt and all three sidecars verified | 12s verified acquisition, 20s packaging, 52s whole job; production job skipped | Excludes publication timing; future tag total unmeasured | PASS |
| Cold and warm audit behavior | CI audit job and targeted job rerun | [Run 42984459656](https://github.com/QubeTX/qube-network-diagnostics/actions/runs/42984459656), jobs 130402220226 and 130404510308 | 204s cold versus 18s warm; exact cache/version, fresh advisory fetch and dependency scan | Existing allowed warnings remain; cache saves runner time, not necessarily critical-path time | PASS |
| Review and exact-head hosted gates | Independent security/installer review; all PR check results | PR49 head ecab180fb76065c83044fdcabc45e272d844466d; CI 42984459656, Mac 42984459602, parity 42984459755, plan 42984459663 | All applicable checks passed before merge | No new product release requested or required | PASS |
| Main delivery | PR merge and exact-main CI/publisher receipts | PR49 merged as 4fe83c1cde62b3f3042a7b371e9b1cdb18be546d; CI 42985659620, parity 42985659478, publisher 42986180521 passed | Workflow changes delivered; publisher skipped existing 4.0.2 | No new crate/tag or product changes | PASS |

## Status
Complete. PR49 is merged, exact-main CI passed, and the publisher successfully skipped existing 4.0.2. Nonpublishing installer proof, cold/warm audit proof, independent review and exact-head hosted checks passed. Measured receipts and limits are in docs/ci-release-efficiency-2026-11-11.md. Future tag end-to-end runtime remains unmeasured; no validation gate was removed. The original security release and homepage deployment are complete separately; no new product version or public artifact changes were made for this optimization.

## Activity
- 2026-11-11 — operator authorized measured CI/CD simplification; selected verified Windows binary reuse and pinned audit-tool caching, preserving release validation (agent: codex)
- 2026-11-11 — implemented and independently reviewed both changes; hosted installer proof and exact-source outputs passed; audit cold/warm jobs measured 204s/18s with fresh advisories; PR49 merged after all exact-head checks passed; awaiting main receipts (agent: codex)
- 2026-11-11 — exact-main CI/parity and idempotent publisher passed at 4fe83c1; all verification and subtasks complete; closed with observed timings and explicit future-tag limits (agent: codex)
