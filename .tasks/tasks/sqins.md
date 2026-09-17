TT;DR: Improve SpeedQX results across CLI, website and native app using the supplied reporting analysis; validate and bump each product version, then upload an iOS candidate if the release checks pass.

## Why
Operator requested cross-surface improvements on 2026-09-16. Existing reports bury load-induced delay, sampling limits and data accounting behind throughput headlines.

## Scope
Expose measured latency deltas, tails and sample counts; clarify repeatable throughput, variation, probe failures and next diagnostic comparisons. Preserve Methodology 5 measurement semantics and existing consent, budgets and native lifecycle. Adaptive saturation and early stopping require controlled accuracy qualification, not an unvalidated reporting change.

## Acceptance
Consistent truthful results on all three surfaces, regression checks, readable responsive presentation and incremented product versions. App upload is authorized; native physical qualification and comparative accuracy remain separately open.

## Verification
- [x] CLI checks and result regressions pass
- [x] Website checks and responsive presentation pass
- [x] Native checks and canonical source parity pass
- [x] Versions and paired changelogs updated
- [ ] Release candidate delivery verified or exact blocker recorded

## Status
Implementation and local checks pass. Website PR #11 is running CI. Native version 3.2.0 bundle exports and upload preparation are underway; CLI version 4.0.2 is ready for PR validation. Native physical qualification and comparative accuracy are not established by these reporting changes.

## Activity
- 2026-09-16 — codex: Read supplied analysis and current Methodology 5 implementations; started scoped cross-surface work.

- 2026-09-16 — codex: 333 Rust tests, strict Clippy, 94 website tests and three-browser controlled UI validation passed. Native tests and pinned source checks passed; full copy includes all result fields and method definitions.

- 2026-09-16 — codex: CI found newly disclosed RUSTSEC-2026-0285. Updated rustls to 0.23.45 and webpki to 0.103.15; cargo audit, all 333 tests and strict Clippy pass locally. Website 4.0.7 is live with exact SHA-256 match. App PR #8 merged; 3.2.0 (20) is building with automatic submission scheduled.

