# SpeedQX 4.0.1 publisher repair — independent delta review

Date: 2026-09-05. Reviewer: independent Codex GPT-6 Astra agent.

Scope: uncommitted changes on `codex/speedqx-v5-publish-fix` against `c95d8dcafaa9b6d6153456d6a9a0c6b4bf5ef07e` (`origin/main` at review). Reviewed the complete nine-file diff: the `phi` helper, crate-publisher toolchain, package version/lock metadata, paired changelogs, agent documentation, and task handoff. No web/app runtime change is included in this delta.

## Findings

No security or arithmetic-correctness defect found in this delta.

- `src/speedtest/stat_primitives.rs:177-200`: the late initialization is replaced by an `if` expression bound to the same explicitly typed `f64`. All arithmetic operands, constants, operation order, comparison thresholds, branch selection, and final sign handling remain identical. A local read-only comparison transformed the base file using exactly the three expected syntactic replacements and asserted byte equality with the current normalized source. This is an assignment-to-expression cleanup, including the existing edge-case behavior; no statistics algorithm changes are hidden in the patch.
- `.github/workflows/crates-publish.yml:38`: the only workflow change is `dtolnay/rust-toolchain@stable` to `dtolnay/rust-toolchain@1.97.0`. An exact normalized comparison confirmed no other change. The successful-main-CI/push/same-repository gate, exact CI head checkout, `contents: read`, secret-scoped publishing steps, concurrency behavior, and registry existence check are unchanged. Pinning the compiler aligns the publisher with the stated validated toolchain without broadening trust or permissions.
- `Cargo.toml:3` and the matching `nd300` entry in `Cargo.lock`: only the package version advances from 4.0.0 to 4.0.1. Exact comparisons confirmed that no dependency version, feature, registry, checksum, or lockfile package relationship changed.
- Documentation describes the publisher failure as occurring before publication and retains 4.0.0 as unpublished. The release statements are intended final-release copy; successful 4.0.1 publication and assets must still be confirmed operationally before announcing completion.

## Validation and limits

Read-only source/diff review and exact transformation checks passed. No product files were edited and no public traffic, publication, or deployment was performed by this reviewer. The parent agent is running compiler/tests and hosted release verification; this review does not substitute for those results. The unchanged v5 Medium/Low security follow-ups from the previous full review remain open. This narrow patch neither introduces new findings nor resolves those earlier notes.

**PASS** for the publisher-repair delta. The prior full v5 verdict remains **PASS WITH NOTES**. Any runtime/workflow change beyond the reviewed expression and toolchain pin requires another delta review.
