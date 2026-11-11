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
- [x] Required local and exact-head CI/release preparation checks pass for the recovery correction
- [x] Exact published 4.0.2 SHA passes main CI, crate publication and signed Mac qualification
- [x] Existing immutable release passes all platforms, exactly 32 assets, checksums, attestations and native trust/bridges
- [x] Homepage fallback is deployed and verified
- [x] Original PRs are merged or explicitly superseded with evidence

## Status
The 4.0.2 CLI release is complete and latest: 32 verified public assets, unchanged original28, all exact-source/tag attestations, both native package lifecycles and both old-updater compatibility bridges. Original root run42972625036 remains failed; successful repair42982227834 attempt3 supplies separate Mac evidence. Real reusable preflight42981719549 passed; main d7c0d50 CI/signing passed and re-enabled publisher42982360234 skipped the already-published crate. Homepage PR23 merged56b2c56; production deployment dpl_EjEk4wmkDGz9ftMRfZkU5E2ZNkrM is READY at that SHA and Chrome verified live4.0.2, directPKG selection and zero console warnings/errors. All release gates are complete. Detailed receipts: docs/release-evidence-v4.0.2.md.

## Activity
- 2026-11-10 — started current-source investigation and authorized deployment scope; confirmed #42 remains vulnerable on main and rejected original incompatible escaping strategy (agent: codex)
- 2026-11-10 — operator expanded scope to #43–46 and authorized GPT-6.1 Sol/xhigh reviewers; assigned isolated file ownership for implementation (agent: codex)
- 2026-11-10 — completed evidence-based dispositions, fixed reproduced interpreter environment channels, cleared 11 current dependency advisories, passed Windows Rust checks and all four installer builds; final hosted qualification/publication still open (agent: codex)
- 2026-11-10 — deliberately retired the optional PowerShell 7 fallback, completed independent platform/automation review and repeated final Rust checks successfully; preparing exact-source PR validation (agent: codex)
- 2026-11-11 — all exact-head workflows passed; merged #42 through the documented administrator publisher PR route without weakening the separate mandatory CI rule. Closed #43–46 with current-source dispositions; started exact-main qualification and retained publication gates (agent: codex)
- 2026-11-11 — exact-main CI and crates publication passed; public crate checksum/source and isolated install verified. Apple qualification attempt 1 failed only on a lost CloudKit ticket-validation connection after both notarizations were Accepted. Started a complete attempt 2 as required; no code/version change or gate waiver (agent: codex)
- 2026-11-11 — full Apple qualification attempt 2 passed all nine required jobs including both native lifecycles; helper confirmed exact-source qualification. Pushed unused immutable v4.0.2 once at ceb10bf; public release verification remains open (agent: codex)
- 2026-11-11 — release builds passed all six targets; fresh Intel public trust check returned Unnotarized Developer ID despite valid checksum/signature and prior exact-byte notarization/Gatekeeper acceptance. Retain the gate and retry the read-only public check after the active Windows job completes; no asset/tag mutation (agent: codex)
- 2026-11-11 — confirmed required explicit reusable-workflow secret interface from current GitHub documentation; both Mac preflights failed empty, while Intel public trust retry passed unchanged bytes. Preparing forward 4.0.3 correction and revised homepage fallback; retained protected environment and exact-source gates (agent: codex)
- 2026-11-11 — 4.0.3 local gates and all five exact-candidate workflows passed; merged PR #47 at 1364ccc and started exact-main qualification. Preserved complete 4.0.1 latest downloads during the correction (agent: codex)
- 2026-11-11 — operator explicitly paused publication and requested investigation before further version churn. Stopped main CI/automatic crate publication and non-publishing qualification; confirmed no 4.0.3 tag or crate. Auditing existing immutable-tag repair route and prepublication coverage gaps; latest remains 4.0.1 (agent: codex)
- 2026-11-11 — operator resumed testing and deployment in execution mode. Withdrawing unused 4.0.3 version bump and implementing publishing-free reusable preflight before same-version 4.0.2 repair; no retagging or existing-asset overwrite (agent: codex)
- 2026-11-11 — real reusable preflight and same-version repair passed; verified all32 public digests/source-ref attestations and unchanged28, both native lifecycles/bridges; preserved failed root record and documented Apple transport-only retries. Homepage merged; production verification pending (agent: codex)
- 2026-11-11 — homepage production READY at56b2c56 and live browser verification passed; completed security release and delivery work. Separate CI efficiency task cieff retains its own verification gates (agent: codex)
