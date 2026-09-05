TT;DR: Implement the user-approved SpeedQX v5 plan across the CLI, website, and app. In progress on codex/speedqx-v5; app branch starts from a536e7b and preserves its one-screen redesign.

## Why
Current v4 upload selection, differing counter semantics, unsupported confidence claims, and provider-mix differences undermine sustained-speed comparability.

## Scope
Shared acquisition contract, primary Cloudflare/MSAK sustained measurement, separately labeled NDT7 and supplementary providers, bounded resources, redesigned cassette and consistent UI. Existing services only. Machine Reports interface is excluded. User subsequently authorized push, merge, production deployment and a new Expo iOS version; App Store review submission remains with the user.

## Plan
Contract and regressions -> acquisition and parity -> interface and local verification -> physical/platform qualification.

## Impact
Methodology becomes 5.0; current headline aliases become sustained measurements. Old results retain v4 meaning. User state, consent, and installation channels are preserved.

## Verification
- [x] TS and Rust replay the same traces including stalls, counter resets, irregular intervals, and partial transfers (26 complete traces; implementation candidate, final-SHA rerun required)
- [ ] Time/byte limits, cancellation, and foreground guards tested
- [ ] Web/app shared source hashes and tests pass
- [ ] Local browser visual states and accessibility checks
- [ ] Platform CI and physical iOS/Android acceptance (not implied by browser preview)
- [ ] Controlled v4/v5 accuracy and animation-impact evidence

## Status
Active: review candidates use a shared committed TypeScript contract and independent Rust acquisition. Canonical website commit efd4f4305dd8de5325ab99848f9fcb99d2764931 is pushed in draft PR QubeTX/speedtest#3. The app candidate a92b554 is pushed in RealEmmettS/speedtest-app-expo#3 and preserves the existing mobile redesign. Local website 87 tests, app 38 tests and Rust 332 tests pass. Paced application references and browser animation comparisons are recorded; physical/device and packet-shaped acceptance, native UI verification, hosted platform checks and release remain open.

## Activity
- 2026-09-04 — codex: created from explicit implementation request; starting measurement trace contract and regression cases.
- 2026-09-04 — codex: implemented sustained accounting, primary-network median, repeatable windows, bounded collection, consent and partial-result ownership. Added matching vector cassette and v5 details. Production authorization recorded; no release claimed.

- 2026-09-04 — codex: paced loopback server records validate browser/Rust payload accounting; Firefox initial upload overshoot reproduced and fixed by limiting growth to 2x, with three repeated asymmetric comparisons under 3% error. Full WebKit matrix under 2%. These are application-level references, not physical packet-shaper evidence.
- 2026-09-04 — codex: user-reported stop/restart crash reproduced as an unhandled asynchronous Expo wake-lock release. Added per-run idempotent leases; 35 app tests pass. Live Expo browser tests pass stop/restart during latency, download and upload with no page errors; probe pulse and reel rotation verified. Stopped runs now say STOPPED and the cassette remains usable.

- 2026-09-04 — codex: candidate versions prepared (CLI 4.0.0, website 4.0.0, app 3.0.0), with paired changelogs and v5 architecture/docs. Website has 87 passing tests; app 35, Expo Doctor 20/20, iOS/Android exports pass. SDK-aligned React Native removes high Metro advisories; remaining app moderate upstream advisories disclosed. Rust audit cleared via compatible h2/chacha20 updates; 331 tests passed before final small contract assertion. No production publication.
- 2026-09-04 — codex: Expo workflow 01a06f6f-f68d-776d-897a-ef268f57f394 built iOS simulator successfully; Android and UI journeys in progress. iPhone 17 slot failed before app launch because the runner lacks that simulator; slot changed to supported iPhone SE with same-build acceptance rerun. Physical-device acceptance remains open.

- 2026-09-04 — codex: final browser/Rust paced matrices each stay within 5% of delivered-payload references; Chromium MAPE 0.7102%, Firefox 0.5650%, WebKit 0.6016%. Original v4 Cloudflare transport comparisons and held-out correlated repeatability experiments are retained with limitations in website evidence/v5/README.md. No physical line-capacity or calibrated 95% accuracy claim is made.
- 2026-09-04 — codex: inspected website idle/completion/settings screenshots after all three browser UI journeys passed. Native iOS/Android builds succeeded, but UI suites found harness issues and an independently reproduced iOS metric-sheet presentation failure. A native-host lifecycle correction and regression test are in app a92b554; fresh workflow 01a06fa5-afae-78bf-aed6-eb7ae5625960 is validating four iPhone sizes and Android. No failed suite counted as passed.
- 2026-09-04 — codex: both generated source manifests now pin clean canonical efd4f4305dd8de5325ab99848f9fcb99d2764931. Final local Rust fmt, strict Clippy, 332 tests, release build, audit, dist plan, workflow and shell syntax checks passed; package/publication and hosted checks remain pending. Production merge/deployment remains conditional on the user's acceptance requirements.
