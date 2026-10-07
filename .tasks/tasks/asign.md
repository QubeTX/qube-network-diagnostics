TT;DR: Keep Apple releases automatic while preventing unreviewed PR code from receiving signing credentials or production signatures. Implementation and GitHub protection setup are in progress; native evidence and credential cutover remain open.

## Why
Operator authorized the implementation plan on 2026-10-06 for Codex Security finding `commit:4555afb7100881918b4c65a5fd9862ff`, PR workflow exposes Apple signing credentials. The original generated patch predates the direct PKG and 32-asset pipeline. Current main `b480a4e7b41d2f490622b56a9362e4f44f320b1d` still signs same-repository PR code.

## Scope
Change workflow authorization, GitHub ref/environment protection, seven existing Apple secret locations, regression tests, and release guidance. Preserve the two-push release process, no recurring approvals, paired binaries, native lifecycle checks, direct PKG, compatibility DMG, checksums, attestations, and immutable 32-asset publication. Do not rotate credentials, alter CLI behavior, touch unrelated PR #38, create a release, or modify existing release assets. Production publication proof is explicitly assigned to #asrel for the next normal release, as accepted in the operator plan.

## Plan
Validate source-bound workflow changes and the temporary public-key-only secret migration. Merge only after required CI and a separate bypass/regression review. Migrate existing secret values directly from a trusted runner to GitHub environment encryption, verify environment metadata and signed qualification, delete repository copies, repeat qualification, remove temporary migration artifacts/workflow, and close the finding with exact evidence.

## Impact
PRs retain native builds without Apple credentials. Every protected main push automatically tests both signed archive paths and the universal installer on Intel/ARM. Release tags wait up to 60 minutes for exact-source qualification. Incorrect environment or dependency wiring can block releases, so archive and installer consumers must be qualified together. Old immutable workflow revisions cannot use environment-only credentials; repairs use hardened tags.

## Acceptance
Functional bar: production credentials cannot reach PR/branch runs; main qualification and future tag publication remain automatic. Evidence bar: local policy/lint checks, exact-head hosted CI, live environment-denial evidence, both native archive and PKG/DMG lifecycles after repository-secret removal, and verified source/settings readback. Gate owner: operator-approved plan and repository release policy. Missing required evidence leaves the finding open. Investigate failures with concrete logs; after two equivalent attempts without information gain, change the experiment and record the blocker.

## Evidence
| Criterion | Oracle | Result | Limitation | Status |
|---|---|---|---|---|
| Local policy and syntax | Python unittest, actionlint, ShellCheck, diff check | 14 tests pass; workflow and shell lint pass | Hosted qualification pending | PASS |
| Environment ref policy | GitHub environment API | apple-signing permits main branch and v* tags, no reviewers or wait | Empty until credential migration | PARTIAL |
| Protected refs | GitHub ruleset API | 24618672/24618673 protect main; 24618674/24618675 protect release tags | Live PR merge still to verify | PARTIAL |

## Verification
- [ ] Exact-head CI, actionlint, ShellCheck and security policy regressions pass
- [ ] Live GitHub environment/ref denial checks pass and all seven Apple secrets exist only in apple-signing
- [ ] Exact-main qualification signs both archives and passes universal PKG/DMG lifecycles on Intel and Apple Silicon after repository-secret removal
- [ ] Migration workflow/ciphertext removed and Codex Security finding closed with exact evidence

## Status
Active. Isolated branch codex/secure-apple-signing starts at current main. Four GitHub rulesets and the empty restricted environment exist. Implementation and a separate bypass/regression review pass are complete; PR, migration and native runs are next. Review traced PR/reusable/dispatch entry points, exact checkout and same-run artifacts, both archive and package secret consumers, output injection, immutable assets, and old-tag behavior. Manual repairs now also require exact-SHA qualification. Original checkout and PR #38 are untouched. Applicable repository policy allows only Opus/Sonnet subagents, unavailable through this harness; the fix-finding skill's separate review-pass fallback was used instead of substituting a model.

## Activity
- 2026-10-06 — PR #39 at f64e11a passed policy/lint/dist/parity but required audit found newly published RUSTSEC-2026-0285 in inherited rustls 0.23.36. Narrowly updated rustls to patched 0.23.45 and required webpki 0.103.15; local cargo audit and locked metadata pass. Live branch dispatch 37554295264 was rejected before every signer, as expected (agent: codex)
- 2026-10-06 — created from the operator-approved implementation plan; confirmed current main and 32-asset contract (agent: codex)
- 2026-10-06 — added source-bound signing jobs, exact-SHA qualification wait, signed archive qualification, policy regressions, environment and ref restrictions; first 14 policy tests and lint checks pass (agent: codex)
- 2026-10-06 — separate bypass/regression pass added the qualification gate to manual repairs; 14 policy tests, actionlint and ShellCheck pass again. Added a temporary no-real-secret branch probe for independent GitHub environment enforcement, and dummy sealed-box round-trip before migration reads credentials (agent: codex)
