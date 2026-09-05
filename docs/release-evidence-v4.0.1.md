# ND-300 4.0.1 and SpeedQX v5 release evidence

This record distinguishes published artifacts, source/CI checks, native fixture acceptance and deferred physical/accuracy qualification. The owner directed deployment on September 5, 2026 after the missed repeatability targets and physical-device gaps were disclosed. Those gaps remain follow-ups, not passed checks.

## Source review and validation

- CLI PR [#33](https://github.com/QubeTX/qube-network-diagnostics/pull/33), candidate `0e67ab32c0e25e76a599f637b94f9486b6bbccf2`, merged as `c95d8dcafaa9b6d6153456d6a9a0c6b4bf5ef07e`.
- Final candidate [CI 33952256757](https://github.com/QubeTX/qube-network-diagnostics/actions/runs/33952256757), [complete trace parity 33952256740](https://github.com/QubeTX/qube-network-diagnostics/actions/runs/33952256740), [native signed/notarized macOS package lifecycle 33952256758](https://github.com/QubeTX/qube-network-diagnostics/actions/runs/33952256758) and [Windows installer origin matrix 33952256785](https://github.com/QubeTX/qube-network-diagnostics/actions/runs/33952256785) all passed.
- Local validation: 332 Rust tests, strict Clippy, formatting, optimized build, audit, isolated package/publication dry-run, dist/workflow/shell checks; 90 website and 38 app tests. All estimate fields and aggregates agree across 88 complete Rust/TypeScript traces.
- [Independent security review](evidence/v5/security-review.md): PASS WITH NOTES, no Critical/High findings. Medium provider metadata bounds, Low endpoint validation and inherited app advisory follow-ups remain at task `sqsec`. The failed Claude automation produced no review and is not counted as a passing check.
- Canonical TypeScript pin `19c83fa484cb65ba4d529d48cc5966cdd106f416` updates release documentation only. Generated app runtime files match the signed build; source hashes and documentation checks pass.

## Publisher compatibility repair

The first publisher run [33953114542](https://github.com/QubeTX/qube-network-diagnostics/actions/runs/33953114542) selected Rust 1.98 through its floating stable toolchain and failed on needless_late_init before publication. No 4.0.0 crate or tag was created. [PR #34](https://github.com/QubeTX/qube-network-diagnostics/pull/34), reviewed head 4ffba1e0b4deb50fd6026e3fc4d55ff9acf8c2af, pins the publisher to the CI/release Rust 1.97.0 toolchain, applies an equivalent phi expression, and advances the CLI to 4.0.1. The [independent delta review](evidence/v5/publish-delta-review.md) passed with exact arithmetic and permission equivalence. Local 332 tests, strict Clippy, optimized build, audit, dist/workflow checks and an isolated packaged-crate publish dry-run passed.

The repair merged as d76af2b2e282440e2e28c31c8b05d5d8833f458f. Exact-head [CI 33953629637](https://github.com/QubeTX/qube-network-diagnostics/actions/runs/33953629637), [parity 33953629677](https://github.com/QubeTX/qube-network-diagnostics/actions/runs/33953629677), [native Mac lifecycle 33953629592](https://github.com/QubeTX/qube-network-diagnostics/actions/runs/33953629592), [Windows origin matrix 33953629664](https://github.com/QubeTX/qube-network-diagnostics/actions/runs/33953629664) and [release plan 33953629948](https://github.com/QubeTX/qube-network-diagnostics/actions/runs/33953629948) all passed. PR #34 had no unresolved review threads.

## CLI publication

Exact-main [CI 33954215496](https://github.com/QubeTX/qube-network-diagnostics/actions/runs/33954215496) and [parity 33954215507](https://github.com/QubeTX/qube-network-diagnostics/actions/runs/33954215507) passed at `d76af2b2e282440e2e28c31c8b05d5d8833f458f`. [Crate publisher 33954398073](https://github.com/QubeTX/qube-network-diagnostics/actions/runs/33954398073) succeeded. Public crates.io readback confirms nd300 4.0.1, unyanked, checksum `100a9f5bfd88e8907603cc0f442547f5ae88fb3c2e76f83f85e5261ed5127a8d`. A fresh isolated cargo install from the public registry succeeded and both commands report 4.0.1, without changing the active installation or PATH. Tag v4.0.1 was created once at the exact merge SHA.

[Release workflow 33954529791](https://github.com/QubeTX/qube-network-diagnostics/actions/runs/33954529791) completed successfully, including all six platform targets, Windows installers, native Intel/Apple Silicon package lifecycles and both immutable v3.7.2 compatibility transitions. The native checks prove Apple trust and byte identity between the direct PKG and the PKG inside its compatibility DMG. Candidate-only jobs were correctly skipped on the tag-triggered run; both release lifecycle and compatibility rows passed. The complete [workflow record](evidence/v5/public-release-workflow.json) retains exact job IDs and step outcomes.

The [public release](https://github.com/QubeTX/qube-network-diagnostics/releases/tag/v4.0.1) contains exactly 32 assets. A fresh independent download verified every GitHub SHA-256 digest, all 13 sidecars, all eight nonblank `sha256.sum` entries, and all 32 attestations against source `d76af2b2e282440e2e28c31c8b05d5d8833f458f` and `refs/tags/v4.0.1`. [Asset inventory and hashes](evidence/v5/public-release.json). The freshly extracted Windows archive contains one `nd300`/`speedqx` pair; both report 4.0.1 and expose `--deep`, `--max-bytes`, and `--accept-mlab`. [Isolated archive smoke](evidence/v5/public-windows-smoke.json). No active installation or PATH was changed.

## Website and homepage

Website [PR #3](https://github.com/QubeTX/speedtest/pull/3) merged as `4ef301aec341741db02ab7fcf16862e530b10843`. [Main CI 33952962972](https://github.com/QubeTX/speedtest/actions/runs/33952962972) and final candidate packet-impairment acquisition run 33952151270 passed. Production Vercel deployment `dpl_6ur7NTAmdYS7e5DZPiRtxqs6mbJJ` is READY at [speedqx.com](https://speedqx.com) with the exact merge SHA. Its public cassette, Methodology 5.0 page and metric explanation open/close were verified; no additional public measurement was started. The earlier loss-repeatability failures remain recorded below.

Homepage [PR #15](https://github.com/QubeTX/qube-machine-report-homepage/pull/15) merged as `c3b152c2683b185a181162a2489c808ce1b6eaaf` after complete CLI publication verification. Vercel deployment `dpl_3YcdxECmf6omAqS4gBc5B3PLk86V` is READY in production at that exact SHA and serves [reports.qubetx.com/nd300](https://reports.qubetx.com/nd300). Local lint/build and preview checks passed. The live page and screenshot confirm the new Quick/Deep, M-Lab consent and payload-ceiling documentation; the four ND300 fallback values are 4.0.1. The established interface is preserved.

## iOS App Store Connect delivery

- App PR [#3](https://github.com/RealEmmettS/speedtest-app-expo/pull/3), candidate `a519c81a28ec225da808782af3e034f58aaa1119`, merged as `5ad0832b98fb50c3ef4291ff506034fcceb32060`. [Main CI 33952381909](https://github.com/RealEmmettS/speedtest-app-expo/actions/runs/33952381909) passed.
- Signed production iOS **3.0.0 (16)**: [EAS build cdc16452-50e4-405b-8857-c0c76735e6a8](https://expo.dev/accounts/realemmetts/projects/speedqx/builds/cdc16452-50e4-405b-8857-c0c76735e6a8), source `0756b10f44d164005cde3137740e494f55bdf7be`.
- [Submission e592005a-2855-4500-99f3-6c4483748b0d](https://expo.dev/accounts/realemmetts/projects/speedqx/submissions/e592005a-2855-4500-99f3-6c4483748b0d) was rechecked through authenticated EAS SDK queries at 2026-09-05T07:21:20Z: FINISHED, IOS, App Store Connect app `6760538784`, no submission error. No duplicate submission was made.
- Archive inspection: `com.speedqx.app`, version/build 3.0.0/16, minimum iOS 15.1, existing encryption declaration, OTA disabled, production UI Lab disabled. IPA SHA-256 `954291d117adbf0ba0983cdf5d3abda627259104f0744526c811570a88773754`.
- Apple processing and physical installation are not independently confirmed. App Store review submission remains with the owner. [TestFlight](https://appstoreconnect.apple.com/apps/6760538784/testflight/ios).

## Native fixtures and deferred qualification

All 30 first-attempt native journeys passed: six each on iPhone SE (third generation), iPhone 16, 16 Plus, 16 Pro Max and Pixel 6 emulator. The archived screenshots and reel recording were visually inspected. These fixtures validate UI behavior in those runtimes, not physical acquisition or radio accuracy.

The optimized paced fixture retained 72 directional runs, with maximum individual payload-accounting error 3.53%. The original WebKit stalled-upload pair differed by 5.33%; three predeclared follow-ups remain alongside it, giving a four-pair median of 1.20%. The optimized packet-shaped run 33948862496 passed acquisition integrity and clean/asymmetric/high-RTT median comparisons, but independent random loss produced 35.11% download and 11.95% upload median differences. Three uncontrolled live Cloudflare pairs also showed variable upload results. No physical-capacity, calibrated 95% coverage or universal device-accuracy claim is made.

Physical accessibility, lifecycle and animation checks remain at `sqphys`; lossy-path and consenting real-provider qualification remain at `sqacc`. Historical failure evidence is retained in the [canonical report](https://github.com/QubeTX/speedtest/blob/19c83fa484cb65ba4d529d48cc5966cdd106f416/evidence/v5/README.md). The release authorization defers these gates without marking them successful.
