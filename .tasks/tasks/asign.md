TT;DR: Keep Apple releases automatic while preventing unreviewed PR code from receiving signing credentials or production signatures. PR #39 is merged and the seven environment credentials are populated. Native signed qualification and removal of repository copies remain open.

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
| Local policy and syntax | Python unittest, actionlint, ShellCheck, diff check | 15 tests pass; workflow and shell lint pass | Signed qualification pending | PASS |
| Exact-head PR validation | CI 37554619098, unsigned Mac 37554619143, Windows origin 37554619072 attempt 2, parity 37554619083, release plan 37554619343 | All successful at 37da38d5afdcc59e2ec485ed5990d11ed246992e | Windows retry recovered an unauthenticated API rate limit; no bypass | PASS |
| Environment ref policy | GitHub environment API and dummy-only run 37555792637 | Branch rejected by protection rules; job 112581596924 had runner_id 0 and no steps | No real credential was referenced | PASS |
| Source gate denial | Branch dispatch 37554295264 | Source rejected and all signing jobs skipped | Other adversarial contexts have local regression coverage | PASS |
| Protected refs | Rulesets 24618672/24618673/24618674/24618675 and PR #39 | Administrator PR merge succeeded after exact-head CI; main force/delete and release tag rewrite/delete forbidden | No additional reviewer or deployment wait configured | PASS |
| Credential migration | Run 37556321121 at 4e07898ffddd923a071c927c4c32ad4c1568baab | Seven environment names verified after pinned-public-key ciphertext import | Repository copies retained until first signed qualification passes | PARTIAL |

## Verification
- [x] Exact-head CI, actionlint, ShellCheck and security policy regressions pass
- [ ] Live GitHub environment/ref denial checks pass and all seven Apple secrets exist only in apple-signing
- [ ] Exact-main qualification signs both archives and passes universal PKG/DMG lifecycles on Intel and Apple Silicon after repository-secret removal
- [ ] Migration workflow/ciphertext removed and Codex Security finding closed with exact evidence

## Status
Active. PR #39 merged at 4e07898ffddd923a071c927c4c32ad4c1568baab. Seven environment secrets were imported; main qualification run 37556320620 was cancelled during initial setup and restarted in full after the environment was populated. Delete repository copies only after this qualification succeeds, then repeat all jobs to prove no fallback. Cleanup is prepared separately and must not merge before that proof. Review traced PR/reusable/dispatch entry points, exact checkout and same-run artifacts, both archive and package secret consumers, output injection, immutable assets, and old-tag behavior. Original checkout and PR #38 remain untouched. Applicable repository policy allows only Opus/Sonnet subagents, unavailable through this harness; the fix-finding skill's separate review-pass fallback was used instead of substituting a model.

## Activity
- 2026-10-06 — merged PR #39 after every relevant exact-head run passed; proved GitHub environment denial with a dummy-only branch job, imported seven existing values using ciphertext-only migration, and restarted main qualification after import. Prepared temporary-workflow cleanup and stable-name candidate-artifact replacement for full retries; public release assets remain immutable (agent: codex)
- 2026-10-06 — PR #39 at f64e11a passed policy/lint/dist/parity but required audit found newly published RUSTSEC-2026-0285 in inherited rustls 0.23.36. Narrowly updated rustls to patched 0.23.45 and required webpki 0.103.15; local cargo audit and locked metadata pass. Live branch dispatch 37554295264 was rejected before every signer, as expected (agent: codex)
- 2026-10-06 — created from the operator-approved implementation plan; confirmed current main and 32-asset contract (agent: codex)
- 2026-10-06 — added source-bound signing jobs, exact-SHA qualification wait, signed archive qualification, policy regressions, environment and ref restrictions; first 14 policy tests and lint checks pass (agent: codex)
- 2026-10-06 — separate bypass/regression pass added the qualification gate to manual repairs; 14 policy tests, actionlint and ShellCheck pass again. Added a temporary no-real-secret branch probe for independent GitHub environment enforcement, and dummy sealed-box round-trip before migration reads credentials (agent: codex)
