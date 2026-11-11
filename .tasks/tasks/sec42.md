TT;DR: Investigate five security pull requests against the current code, repair the still-relevant findings, and ship only after the required release checks pass.

## Why
Direct operator request on 2026-11-10: investigate PR #42, fix conflicts and deploy if still needed; subsequently extended to #43–46. The operator also authorized GPT-6.1 Sol at xhigh for subagents and requested matching AGENTS.md guidance.

## Scope
PR #42 Wi-Fi configuration injection; #43 privileged program resolution; #44 release-tool supply chain; #45 local automation download integrity; #46 direct macOS package trust. Current source baseline: main d9f0d35be5767152d8d3c670a991537d0bc1c7cf. Preserve protected Apple signing, installation ownership and immutable release artifacts. Keep unrelated PR #38 result-insight work separate. Include version/changelogs, homepage fallback, mirrored agent guidance and #asrel publication proof.

## Plan
Review each original patch against current main; implement only remaining issues with tests; independently review security and platform behavior; complete local/CI gates; merge, qualify exact main SHA and publish a new patch tag; verify crate, 32 assets, checksums/attestations, native Mac bridges/trust and homepage.

## Impact
Prevent attacker-controlled Wi-Fi text and executable lookup from influencing privileged repair. Authenticate downloaded automation tools and strengthen package verification without breaking valid installations or future upgrades. Changes to privileged actions and release automation require regression and native checks.

## Acceptance
Functional bar: current vulnerabilities are either demonstrably obsolete or fixed without reverting later hardening. Evidence bar: per-fix regressions and independent reviews, exact-source CI, public release and install verification required by AGENTS.md/release skill. No destructive live network tests. Unavailable evidence remains open, never self-waived.

## Evidence
| Criterion | Oracle / invocation | Result | Interpretation | Limitation | Status |
|---|---|---|---|---|---|
| Relevance #42 | origin/main stages.rs privileged wpa_supplicant append | raw SSID/passphrase remain | fix needed; original C-style escaping changes ordinary quoted values | patch requires tests | PASS |
| Relevance #43–46 | current source, harmless exploit controls, publisher bytes and primary references | `docs/security-pr-review-2026-11-10.md` | #43 needed; #44 partly superseded; #45 hardening needed; #46 unsupported and incompatible | no assertion of observed exploitation | PASS |
| Local validation | Rust 1.97 fmt, strict all-feature Clippy, all-target tests, optimized build, audit | 338 library + 5 CLI tests; zero current audit vulnerabilities | implementation passes Windows checks | hosted platforms remain | PASS |
| Installer/automation checks | four local installer builds, checksum/updater fixtures, Apple policy and shell parser tests | four exit 0 builds; 15 Apple policy tests; parser fixtures passed | formats and protected signing contract preserved | local compilation is not a native install lifecycle | PASS |

## Verification
- [x] Every PR has a documented current-source relevance disposition
- [x] Regression tests, security review and cross-platform review pass
- [ ] Required local and exact-head CI/release preparation checks pass
- [ ] Exact merged SHA passes main CI, crate publication and signed Mac qualification
- [ ] New immutable release passes all platforms, exactly 32 assets, checksums, attestations and native trust/bridges
- [ ] Homepage fallback is deployed and verified
- [ ] Original PRs are merged or explicitly superseded with evidence

## Status
Active in isolated #42 worktree; current main is merged into the old PR without rewriting published history. Relevant #42–45 fixes and the model guidance are implemented; the evidence review rejects #46's wrong certificate pin and unsupported signature-bypass claim. Independent security/platform review cleared the demonstrated injection, executable lookup, Bash startup, PowerShell module and missing-VPN fallback paths. Four installers build. Final review identified the optional PowerShell 7 updater fallback as incompatible with fixed system lookup; it is deliberately retired, documented and covered by 48 updater tests. Final broad checks again pass (338 library + 5 CLI tests, strict Clippy, optimized build); automation review approved all pinned digests and negative integrity controls. Actual Windows cargo-dist download/verify/execution passed in an isolated tool directory. Next: exact-head CI, merge, exact-main Apple qualification/crate, immutable 4.0.2 tag and 32-asset verification. Homepage PR #23 passed preview checks and awaits release delivery. No release gate is waived.

## Activity
- 2026-11-10 — started current-source investigation and authorized deployment scope; confirmed #42 remains vulnerable on main and rejected original incompatible escaping strategy (agent: codex)
- 2026-11-10 — operator expanded scope to #43–46 and authorized GPT-6.1 Sol/xhigh reviewers; assigned isolated file ownership for implementation (agent: codex)
- 2026-11-10 — completed evidence-based dispositions, fixed reproduced interpreter environment channels, cleared 11 current dependency advisories, passed Windows Rust checks and all four installer builds; final hosted qualification/publication still open (agent: codex)
- 2026-11-10 — deliberately retired the optional PowerShell 7 fallback, completed independent platform/automation review and repeated final Rust checks successfully; preparing exact-source PR validation (agent: codex)
