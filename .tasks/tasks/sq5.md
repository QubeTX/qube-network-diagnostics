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
- [x] Full TypeScript/Rust replay: 88 synthetic, recorded Cloudflare and packet-shaped traces, plus repeated-provider and cross-provider ceiling regressions (hosted run 33945077402, source efbe6304057293559e92ddbc723190924c640fc6)
- [x] Deterministic time/byte limits, cancellation, foreground guards and real Expo browser Stop/reset/restart during latency/download/upload
- [x] Shared web/app source hashes pin clean canonical 08d1c337b977d07e2296fe3a2e57f2983afa298f; website 90 tests and app 38 tests pass
- [x] Chrome/Firefox/WebKit visual journeys, narrow widths, metric explanations, trace details and reduced motion
- [x] Rust 332 tests, strict Clippy, release build and isolated package publish dry-run; platform CI 33945077385 and signed/notarized native Mac lifecycle 33945077405 pass
- [ ] Final native UI acceptance
  - [x] Android five-flow suite, workflow 01a06fd9-f476-76ef-9c61-838bfe74e0c3, source 5ec3f6ec7a63f1d3d78baca4a74b0cd840e6e88c
  - [ ] Four iPhone sizes; compact consent-sheet scroll investigation remains open
- [ ] Physical iOS/Android large text, reduced motion, cancellation, network/background transitions and animation overhead
- [ ] Controlled accuracy and cross-surface repeatability: paced payload cases pass; random-loss paired medians exceed 5%, retained with analysis in canonical evidence/v5/README.md
- [ ] Substantive PR review: the existing Claude reviewer fails before any model usage; a workflow-edited apparent success was a validation skip and the original workflow was restored
- [ ] Production merge, immutable CLI release, website/homepage production deployments and accepted Expo candidate; App Store review submission remains with the user

## Status
Active: all four review PRs are pushed. The canonical measurement implementation is committed and byte/time semantics agree across languages. Real Cloudflare and Linux packet impairment recordings are retained, including failed repeatability targets. The stop crash, web font rasterization error and native metric tap interception are fixed; native reels are visible in reviewed simulator recordings. Android passes all five UI flows. The compact iPhone consent sheet and remaining device matrix are being checked. A full signed iOS 3.0.0 build 15 is queued for TestFlight preparation, not published. Physical acceptance and final production release remain open.

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

- 2026-09-04 — codex: recorded three fresh-client netem pairs per condition (33943631436). Clean/asymmetric/high-latency paired medians are below 5%; random-loss medians are 32.79% download and 10.80% upload, with substantial within-surface variability. Three real Cloudflare pairs completed; median differences were 2.20% download and 11.13% upload on the uncontrolled host path. No physical-capacity or all-device accuracy claim is inferred.
- 2026-09-04 — codex: a live trace exposed a 154.27 Mbps ceiling below 159.68 Mbps sustained. Both engines now withhold unsupported lower ceilings rather than raising them. All 88 counter traces and explicit aggregation cases agree after the correction. Package verification uses an isolated target directory because the previous package dry-run left cached library dependencies pointing into the unpacked package; a fresh package-scoped rebuild resolved the stale replay executable.
- 2026-09-04 — codex: restored the original reviewer workflow after confirming edited-workflow validation produces a skip. Its latest real attempt still fails before substantive review. Android full UI workflow succeeds; native iOS recording frames show reel motion and probe feedback. Production build af9b7680-9118-458b-8a68-fdd631205e8a is preparatory only; no merge, release tag, production deployment or TestFlight upload claimed.
