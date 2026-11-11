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

Windows installer add-ons will acquire the exact release ZIP and sidecar, bind tag to source, verify checksum and source/ref/signer provenance, safely extract and version-check both binaries, then build the existing installers. A read-only validation dispatch exercises that acquisition and packaging path without publishing. There is no rebuild fallback if verification fails.

CI will cache the pinned cargo-audit 0.23.0 executable by exact host, architecture, version and Rust 1.97.0 toolchain. It checks the executable version before use and continues the full advisory refresh and audit. Advisory results are not cached or skipped.

Expected savings are approximately four minutes on the release packaging critical path and roughly three runner-minutes on a warm audit job. Audit wall-clock savings depend on whether that job is the critical path. Future tag runtime is not yet measured and must not be presented as demonstrated speedup.

## Deliberately separate follow-ups

Native PR compilation is repeated across workflows with different flags, target layouts and cache identities. Windows Cargo lifecycle builds use separate install target directories. The publisher repeats same-SHA CI checks, and documentation changes can trigger broad builds. Each offers possible savings but requires an independent design so artifact ownership, required-check behavior and real Cargo lifecycle coverage stay trustworthy.

## Verification

Implementation, independent review, hosted validation and final merge receipts remain pending. These claims are not complete until exact-source hosted checks are recorded.
