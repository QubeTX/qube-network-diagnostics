# ND-300 v3.7.3 direct-package release qualification

This record separates reviewed source, immutable publication, fresh public
artifact verification, hosted native-package testing, disposable Windows
journeys, production-homepage deployment, and the final physical-machine
checks. A section remains explicitly pending until its evidence exists.

## Source and pre-release gates

- Pull request: [#28](https://github.com/QubeTX/qube-network-diagnostics/pull/28)
- Reviewed head: `b763ce8883fd69b51fd2f4950c46f64618344763`
- Merge commit and release source: `86d78e7a68c41bdd1bb6a65407d6912381e12724`
- [Exact-head CI](https://github.com/QubeTX/qube-network-diagnostics/actions/runs/29657984654), [native Intel/Apple-Silicon package qualification](https://github.com/QubeTX/qube-network-diagnostics/actions/runs/29657984647), [candidate Windows origin matrix](https://github.com/QubeTX/qube-network-diagnostics/actions/runs/29657984670), [release-plan gate](https://github.com/QubeTX/qube-network-diagnostics/actions/runs/29657984723), and Claude review all passed. The PR exposed 40 successful checks and no failure.
- Cross-platform cfg and four-installer LOCKSTEP reviews reported no findings. Local Rust 1.97.0 qualification passed formatting, strict all-target/all-feature Clippy, 327 tests, release build and smoke, audit, cargo-dist planning, package and publish gates, actionlint, ShellCheck, parser/fault/checksum fixtures, and zero-exit builds of all four Windows installers.

## Immutable publication

- [Exact-main CI](https://github.com/QubeTX/qube-network-diagnostics/actions/runs/29658442612) passed all 18 jobs at the merge commit.
- [Crates.io publication](https://github.com/QubeTX/qube-network-diagnostics/actions/runs/29658557309) succeeded; [`nd300 3.7.3`](https://crates.io/crates/nd300/3.7.3) is public and unyanked.
- Tag `v3.7.3` was created and pushed once at the exact merge commit.
- [Release workflow](https://github.com/QubeTX/qube-network-diagnostics/actions/runs/29658615661) succeeded across all six cargo-dist targets, reusable Windows add-ons, reusable macOS packaging, and final announcement.
- [GitHub release](https://github.com/QubeTX/qube-network-diagnostics/releases/tag/v3.7.3) is public, non-draft, non-prerelease, and names the exact merge commit.

## Public artifacts, provenance, and Apple trust

- Exactly 32 assets are public. A fresh independent download of all 32 assets matched every GitHub-recorded digest.
- All 13 SHA-256 sidecars and all eight nonblank `sha256.sum` entries verified. Both public binaries in the Windows archive report 3.7.3.
- `gh attestation verify` accepted all 32 assets with source digest `86d78e7a68c41bdd1bb6a65407d6912381e12724`, source ref `refs/tags/v3.7.3`, and the expected signer repository.
- Apple accepted both the direct universal PKG and compatibility DMG. The public PKG is signed with the pinned Developer ID Installer identity, notarized, stapled, Gatekeeper-accepted, and contains only the expected universal `nd300`/`speedqx` payload and package metadata. The compatibility DMG is signed, notarized, stapled, Gatekeeper-accepted, and contains the byte-identical public PKG.
- The tag workflow's native Intel and Apple Silicon jobs passed the direct-package install, repair, takeover, explicit downgrade, update selection, receipt/hash ownership, uninstall, and reinstall lifecycle. Its post-publication bridge installed immutable v3.7.2 on both native architectures and proved that the legacy DMG path converges to the v3.7.3 package without a duplicate.

## Public Windows update journeys

- [Public Windows origin matrix](https://github.com/QubeTX/qube-network-diagnostics/actions/runs/29659458550) passed all 12 jobs from fresh GitHub-hosted VMs.
- The matrix covered Global MSI, Corporate MSI, Global EXE, Corporate EXE, Cargo, the v2.9.0 pre-marker Global MSI, all supported same-scope fresh installer takeovers, opposite-scope refusal, and the surviving-Cargo stale-marker case.
- Every row began from public 3.7.1, updated with public v3.7.3 bytes, opened a fresh shell, verified both commands and origin, exercised migration and uninstall behavior, and retained strict one-object JSON on v3.7.2-and-newer updater entrypoints.

## Homepage

- Homepage PR [#11](https://github.com/QubeTX/qube-machine-report-homepage/pull/11) passed lint, production build, local rendered Mac/Windows/install-guide inspection, and both Vercel checks at head `887df904544137695a2cec5bdf5e466dcfd2bfe3`.
- Merge `0ed438ccd45518b51255dcd65f57130b483663b7` received a successful production Vercel status.
- A fresh request to [`reports.qubetx.com/nd300`](https://reports.qubetx.com/nd300) returned HTTP 200. The deployed production bundle contains the 3.7.3 fallback, command-first recommendation, and versionless `releases/latest/download/nd300-universal-apple-darwin.pkg` link.
- The native PKG is an optional explicit channel; the compatibility DMG is not advertised. Windows keeps the versionless `irm` route first and the four latest-download MSI/EXE channels as optional managed or graphical choices.

## Physical Alienware result

The installed 3.7.2 Global MSI selected only `msi-global`, downloaded and
verified the exact public 3.7.3 MSI, received operator-approved UAC, and
completed the real `nd300 update --json` transaction with exit 0. The updater's
one-object response reported 3.7.2 -> 3.7.3, strategy `msi_global`, no remaining
user action, and no cross-channel fallback. Fresh processes then proved:

- `nd300` and `speedqx` both report 3.7.3 from `C:\Program Files\nd300\bin`.
- Their SHA-256 digests are `35776ED3BEE3E3149DA705E7FDB35D6768A98BA65112A13C4AA69E5189C14D93` and `DF366B4158C337BF536FFF4FEF81AA1AAC078A870BA602C6EC8B4F9B36FA6E71`, byte-identical to the independently downloaded public Windows archive.
- Exactly one machine-wide ARP record remains: product `{D0AD3FD5-B99F-4D4A-98B8-C088355E6437}`, version 3.7.3, under `C:\Program Files\nd300\`. Marker `msi-global` and exactly one expandable machine-PATH entry remain; no user-PATH duplicate exists.
- The install directory contains only the two expected binaries. No Cargo copy, retired/update-old sibling, staged updater MSI, or active Windows Installer transaction remains.
- Both `nd300 update --json` and `speedqx update --json` emit exactly one object, exit 0, report current/latest 3.7.3, preserve `msi-global`, and require no action. Migration dry-run finds neither a Cargo copy nor another edition.
- Installed core JSON, technician JSON, and ASCII smokes all exit 0 with empty stderr. JSON parses with the required core/technician structures; ASCII contains no Unicode box drawing. The result identifies exactly one active Intel Wi-Fi 7 BE202 at 866 Mbps and the reachable default gateway while disconnected Wi-Fi Direct, Bluetooth, and Ethernet adapters do not inflate the active count.
- High-confidence credential-value scans are empty. No `fix`, uninstall, service cycle, adapter change, or network mutation was run.

## Physical Mac result

Pending only if the temporary testing Mac is available. Hosted native Intel and
Apple Silicon evidence above is complete and release-blocking. The remaining
physical batch is an additional GUI acceptance of an immutable v3.7.2
DMG-driven update into the direct-PKG era plus a direct-PKG fresh/update smoke;
it must not be represented as complete without an actual machine result.

## Repository closeout

Pending the two physical-machine dispositions. The final closeout PR will name
its exact head/main CI, then the branch/worktree audit will delete only
merged/closed/superseded refs with no unpreserved unique work. ND's untracked
`.agents`, `.codex`, and generated installer output remain outside cleanup, and
the homepage's unrelated dirty SD-300 worktree remains untouched.

## Deferred, non-release-blocking work

- The destructive macOS service-cycle gate remains disabled pending disposable-hardware qualification.
- Full post-finalize cross-edition Windows unregistration remains separately tracked; installer-internal cleanup stays advisory and file-limited.
- The physical read-only diagnostic repeat, shared topology snapshot, and broader Unix updater fault-injection work remain in the post-v3.6 backlog.
