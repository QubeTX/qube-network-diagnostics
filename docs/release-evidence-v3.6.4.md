# ND-300 v3.6.4 release qualification

This record closes the hardened v3.6 release line. It separates reviewed source,
hosted candidate testing, immutable public artifacts, disposable public update
tests, and the physical Windows result so later maintainers can reproduce the
release decision.

## Source and pre-release gates

- Pull request: [#22](https://github.com/QubeTX/qube-network-diagnostics/pull/22)
- Reviewed head: `9cc885106372759534823640ca306360cd139b0d`
- Merge commit: `e3bf6c5f4fbd7200d915d2b67334086af698dea5`
- [Exact-head CI](https://github.com/QubeTX/qube-network-diagnostics/actions/runs/29627244812): all 18 jobs passed across both macOS architectures, Windows, Linux GNU, Linux musl, Linux ARM, ShellCheck, audit, cargo-dist planning, and installer tests.
- [Candidate Unix update matrix](https://github.com/QubeTX/qube-network-diagnostics/actions/runs/29627244820): all five jobs passed.
- [Candidate Windows origin matrix](https://github.com/QubeTX/qube-network-diagnostics/actions/runs/29627244810): all 13 lifecycle, legacy, takeover, refusal, lock, and rollback jobs passed.
- Cross-platform cfg review and four-installer LOCKSTEP review found no blockers at the reviewed head. Local Rust 1.97.0 qualification also passed formatting, strict all-target/all-feature Clippy, 323 Rust tests, release build, audit, the 104-file crate package, publish dry-run, actionlint, ShellCheck, parser/fault/checksum fixtures, and zero-exit builds of all four installers.

## Immutable publication

- [Exact-main CI](https://github.com/QubeTX/qube-network-diagnostics/actions/runs/29627666901) passed at the merge commit.
- [Crates.io publication](https://github.com/QubeTX/qube-network-diagnostics/actions/runs/29627786358) succeeded; [`nd300 3.6.4`](https://crates.io/crates/nd300/3.6.4) is public.
- Tag `v3.6.4` was created and pushed once at the exact merge commit.
- [Release workflow](https://github.com/QubeTX/qube-network-diagnostics/actions/runs/29627840024) succeeded, including the reusable Windows installer workflow and release announcement.
- [GitHub release](https://github.com/QubeTX/qube-network-diagnostics/releases/tag/v3.6.4) is public, non-draft, non-prerelease, and names the exact merge commit.

## Public artifact and Apple trust proof

- Exactly 28 assets are published. Fresh downloads matched all 28 GitHub-recorded SHA-256 digests.
- All 11 sidecars and all eight nonblank `sha256.sum` entries verified. Canonical and legacy shell/PowerShell installer aliases are byte-identical.
- All 28 artifact attestations verified against source digest `e3bf6c5f4fbd7200d915d2b67334086af698dea5` and source ref `refs/tags/v3.6.4`.
- Fresh public Windows archive binaries both report 3.6.4.
- Apple submission `ed0de0a6-4ec6-437b-8304-8214fea6528e` (ARM) and `83e5596e-65b7-43b8-b2e9-18280916432b` (Intel) both report `Accepted`.
- The exact public ARM and Intel pairs pass Gatekeeper with `source=Notarized Developer ID`; public verification jobs `88036261025` and `88036261046` passed.

## Public update journeys

- [Unix public matrix](https://github.com/QubeTX/qube-network-diagnostics/actions/runs/29628429359) passed all five jobs: ARM Mac from the current managed baseline, Intel Mac through `speedqx update`, Linux managed archive, Linux Cargo, and the Linux 3.5.2 legacy boundary.
- [Windows public matrix](https://github.com/QubeTX/qube-network-diagnostics/actions/runs/29628428697) passed all 12 substantive jobs: Global MSI, Corporate MSI, Global EXE, Corporate EXE, Cargo, pre-marker v2.9 Global MSI, Cargo-to-Global-MSI fresh takeover, all four supported same-scope MSI/EXE takeovers, and byte-preserving opposite-scope refusal. The rows also exercise migration, fresh-shell ownership, rollback, and uninstall.

## Physical Alienware result

The installed 3.5.2 Global MSI selected only the verified `msi-global` strategy,
received operator-approved UAC elevation, and completed the public 3.6.4 update
with exit 0. A new PowerShell process proved:

- `nd300` and `speedqx` both resolve only to `C:\Program Files\nd300\bin` and report 3.6.4.
- Their SHA-256 digests are `93A5DF0A0084BE25395B6CC2F5AA208F1BC9394583B0EB4CCAA7621C39C295FD` and `3A2CBF1CAC3DA2559339F7442FE37BAE99EB5C24C96BA59F183A3C38CE325752`, byte-identical to the independently downloaded public Windows archive.
- Exactly one Global MSI ARP record remains: product `{790A5106-49B9-4A96-9B25-23ADE6465E78}`, version 3.6.4. Marker `msi-global` and exactly one machine-PATH entry remain; there is no user-PATH duplicate.
- The old Cargo 3.4.0 shadow is gone, no Cargo `speedqx` exists, no retired/update-old file remains, and the install directory contains only the two expected binaries.
- Both updater entrypoints report 3.6.4 as already latest with origin `msi-global`; migration dry-run reports no Cargo copy or other edition.
- Core and technician JSON parse and exit 0; ASCII exits 0 and contains no Unicode box characters. The result identifies one active Intel BE202 Wi-Fi adapter and the unchanged default route while excluding disconnected/virtual adapters from the active count. Credential/secret/token/key/authorization scans are clean; the sole broad `password` word match is a process name, not credential data.
- Wi-Fi remains up and the default route remains alive. No `fix`, uninstall, service cycle, or network mutation was run.

## Homepage

- Homepage PR [#8](https://github.com/QubeTX/qube-machine-report-homepage/pull/8) passed lint, production build, and Vercel preview at head `dac12a6a9b65a95c3cfe68e332a4adec36e359a9`.
- Merge `fab1628b58c828e5ba55b30ad0ccb684a20f4456` deployed successfully as Vercel production deployment `5498518176`.
- A no-cache request to [`reports.qubetx.com/nd300`](https://reports.qubetx.com/nd300) returned HTTP 200; the production bundle contains the 3.6.4 fallback in all four source call sites and no 3.6.2 fallback. The live GitHub lookup resolves `v3.6.4`.
- Obsolete homepage PR #7 was closed as superseded. SD-300 and Shaughv OS remain intentionally delisted/WIP.

## Deferred, non-release-blocking work

- The destructive macOS service-cycle gate remains disabled pending disposable-hardware qualification.
- Full post-finalize cross-edition Windows unregistration remains a separately tracked design task; installer-internal cleanup stays advisory and file-limited.
- The native signed/notarized macOS `.pkg` channel begins after this closeout and must preserve ADR-0002's channel, exact-version, paired-binary, rollback, and trust rules.
