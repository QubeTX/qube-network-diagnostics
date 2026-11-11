# Release efficiency: measured changes

The operator requested simpler, faster CI/CD while retaining its protections. These changes address repeated compilation; they do not remove correctness, installer lifecycle, signing or provenance checks.

## Baseline measurements

| Observed workflow | Measured duration or cost | Interpretation |
|---|---:|---|
| PR48 CI, run 42979785870 | 4m43s | Critical path was native Intel work, not container images. |
| PR48 native Mac workflow, run 42979785864 | 6m05s | Repeats native compilation after runner queue time. |
| PR48 Windows matrix, run 42979785970 | 15m03s | 5m49s candidate build followed by a 9m09s Cargo case. |
| Windows Cargo case | 4m04s baseline install plus 3m51s local install | Real source compilation dominates; test assertions are not the costly part. |
| Release Windows add-ons, run 42972625036 | 246s recompiling binaries | Same tag/source Windows pair already exists in the release ZIP. |
| CI audit-tool installation | 153–195s | Tool compilation repeats; the audit itself takes seconds. |
| Successful main Mac qualification attempt 2, run 42969643742 | 8m03s actual attempt runtime | Earlier failures, retries and coordination add elapsed time but are not a single normal run. |

## Selected changes

Windows installer add-ons acquire the exact release ZIP and sidecar, bind tag to source, verify checksum and source/ref/signer provenance, safely extract and version-check both binaries, then build the existing installers. A read-only validation dispatch exercises that acquisition and packaging path without publishing. There is no rebuild fallback if verification fails.

CI caches the pinned cargo-audit 0.23.0 executable by exact host, architecture, version and Rust 1.97.0 toolchain. It checks the executable version before use and continues the full advisory refresh and audit. Advisory results are not cached or skipped.

Expected savings are approximately four minutes on the release packaging critical path and roughly three runner-minutes on a warm audit job. Audit wall-clock savings depend on whether that job is the critical path. Future tag runtime is not yet measured and must not be presented as demonstrated speedup.

## Deliberately separate follow-ups

Native PR compilation is repeated across workflows with different flags, target layouts and cache identities. Windows Cargo lifecycle builds use separate install target directories. The publisher repeats same-SHA CI checks, and documentation changes can trigger broad builds. Each offers possible savings but requires an independent design so artifact ownership, required-check behavior and real Cargo lifecycle coverage stay trustworthy.

## Verification

Implementation PR [#49](https://github.com/QubeTX/qube-network-diagnostics/pull/49),
head `ecab180fb76065c83044fdcabc45e272d844466d`, merged as
`4fe83c1cde62b3f3042a7b371e9b1cdb18be546d` on November 11. Independent
workflow/security review approved the implementation. Focused acquisition and
audit-cache fixtures, existing installer helper suites, all-workflow Actionlint,
all-script ShellCheck, 19 Apple signing-policy tests and the secret scan passed.
No Rust product source or package version changed.

The [read-only Windows proof, run 42984456696](https://github.com/QubeTX/qube-network-diagnostics/actions/runs/42984456696),
used the exact candidate tooling with v4.0.2 installer source
`ceb10bf6e5818a36200c8292c78c3202160de864`. Archive acquisition and verification
took **12 seconds**; Corporate MSI and both Inno EXEs took **20 seconds** to
build; the complete validation job took **52 seconds**. Its production job was
skipped. Downloaded workflow evidence contains the exact product/tooling source
receipt and all three expected installers with matching SHA-256 sidecars.
The verified archive digest is
`cf76450b8ae87e067e59509e265f7c45f6ec14faa4123a2b629ca628db3d0bf8`.

[CI run 42984459656](https://github.com/QubeTX/qube-network-diagnostics/actions/runs/42984459656)
passed on the exact PR head. Attempt 1's audit job 130402220226 installed the
pinned tool in **187 seconds** and completed in **204 seconds**. A targeted
rerun of only that job (plus its dependent required-check aggregation) restored
the exact cache key `cargo-audit-v1-Linux-X64-0.23.0-rust-1.97.0`, verified
version 0.23.0, fetched current RustSec advisories and scanned Cargo.lock without
compiling the tool. Job 130404510308 completed in **18 seconds**, saving
**186 seconds** for that observed job. The advisory scan retained the existing
two allowed warning results; neither warning policy nor advisory data was cached.

The same head passed native Mac PR builds in run 42984459602, measurement parity
in 42984459755, and release planning in 42984459663. Every PR check was complete
and successful or intentionally skipped before the administrator publisher PR
route merged it. The separate no-bypass `CI required` gate passed.

At exact merge SHA `4fe83c1cde62b3f3042a7b371e9b1cdb18be546d`,
[main CI 42985659620](https://github.com/QubeTX/qube-network-diagnostics/actions/runs/42985659620)
and measurement parity 42985659478 passed.
[Publisher 42986180521](https://github.com/QubeTX/qube-network-diagnostics/actions/runs/42986180521)
succeeded, confirmed that 4.0.2 was already published, and skipped publication.
This delivers the workflow changes without a new crate or release tag.

The future tag's end-to-end runtime remains unmeasured; the 52-second validation
job excludes publication and does not replace installer lifecycle, signing,
compatibility, provenance or complete-inventory requirements.
