TT;DR: Repair the running-image update transaction, prove `nd300 update` from every supported origin, and upgrade the Alienware's existing Global MSI copy from 3.5.2.

## Why
Builds prove that artifacts exist, not that v3.5.2 selects and installs the correct public 3.6.x artifact. Origin detection, paired binaries, fresh-shell PATH, marker lifecycle, and uninstall must work in real updates.

## Plan
Use the established preserve/replace/restore transaction: verify exact-tag downloads before mutation; on Windows rename a running canonical image to an allowlisted sibling, replace and verify the pair, restore on every failure, and clean the retired image only after the old process exits. Prove one channel before mutation and never cross-fall-back: Cargo, managed Unix archive, Windows standalone, and each MSI/EXE edition stay exact. A fresh official Windows installer may replace a proven MSI/EXE format only within the same per-user/per-machine scope; an opposite-scope owner is refused before mutation. Prove injected rollback plus suspended current and pre-marker v2.9 Global MSI images before merge. Then run disposable public matrices for all five current Windows origins, v2.9, same-scope takeovers, opposite-scope refusal, and supported Unix targets, followed by the Alienware's 3.5.2 `msi-global` installation. Do not use the physical machine for takeover/uninstall tests.

## Impact
This is the final Windows user-journey gate. Failure after tagging triggers the documented patch-bump loop rather than retagging.

## Acceptance
Every supported origin reaches one matched final-version binary pair at the intended location; a failed copy/setup restores both old binaries and registration; the initiating old process is not killed; incorrect/unknown ownership is refused; sidecars validate; uninstall removes only the proven origin; supported Unix updates pass; and the Alienware upgrade succeeds without network-state mutation.

## Verification
- [ ] Global MSI public updater case passes
- [ ] Corporate MSI public updater case passes
- [ ] Global EXE public updater case passes
- [ ] Corporate EXE public updater case passes
- [ ] Cargo public updater and stale-marker case pass
- [ ] Windows locked-image and injected-rollback candidate matrix passes
- [ ] Pre-marker v2.9 Global MSI candidate and public update pass
- [ ] Same-scope Windows channel-takeover candidate and public matrix passes
- [ ] Opposite-scope refusal preserves the original install byte-for-byte
- [ ] Public macOS and Linux updater matrices pass
- [ ] Alienware fresh shell resolves both binaries to the final Global MSI version

## Status
Active. Public v3.6.2 matrix run `29610274431` failed all five origins. Diagnostic run `29611079431` proved Cargo and cargo-dist could not overwrite the running Windows image; installer origins were terminated by Restart Manager/application shutdown before reporting. v3.6.3 applies a channel-preserving, exact-release preserve/replace/restore transaction. PR #21's first exact head exposed three non-Windows warning shapes and a Windows checkout-newline fixture defect; the additive correction is locally green and awaiting the replacement exact-head run. No merge or tag occurs before cross-platform CI and the disposable Windows candidate matrix pass.

## Activity
- 2026-07-17 10:18 — imported post-release portion of ND36-003 (agent: codex)
- 2026-07-17 15:19 — public v3.6.2 five-origin run `29610274431` failed; retained the Alienware's 3.5.2 Global MSI as the physical baseline (agent: codex)
- 2026-07-17 15:47 — diagnostic run `29611079431` captured Windows access-denied replacement and installer-initiated process termination; artifact comparison ruled out the macOS revisions as the cause (agent: codex)
- 2026-07-17 17:08 — selected the Claude Code-style preserve/replace/restore transaction after inspecting official Claude/Codex update layouts; executable live-image and multi-stage rollback fixtures passed locally (agent: codex)
- 2026-07-17 17:32 — finalized the same-channel contract, exact-tag pinning, fresh official-installer takeover, and safe manual-download failure path; 56 focused Rust tests plus the generated PowerShell fixtures passed (agent: codex)
- 2026-07-17 17:45 — all four production installers and four fault variants compiled; MSI table inspection proved rollback fault ordering and commit-only retired cleanup. Hosted coverage expanded to five current origins, v2.9 Global MSI, and five fresh takeover directions (agent: codex)
- 2026-07-17 18:39 — security review removed unsupported cross-context MSI takeover, required Windows-Installer proof for MSI product codes, made the generated PowerShell route fail closed, moved MSI markers and v2.9 PATH repair to commit maintenance, and added byte-identical opposite-scope refusal coverage. Local Rust and four production installer gates passed; final fault rebuild and hosted candidate proof remain (agent: codex)
- 2026-07-17 18:51 — frozen local candidate passed format, strict all-target/all-feature Clippy, 321 Rust tests, release build, audit, 104-file package and publish dry-run, cargo-dist plan, actionlint, eight tracked-blob ShellCheck files, parser/checksum/PowerShell fixtures, all four production installers, and all four rollback-fault installers. Cross-platform cfg verdict is likely green and installer lockstep verdict is green; hosted exact-head proof is next (agent: codex)
- 2026-07-17 18:59 — PR #21 run `29621842015` rejected the first exact head for two Unix-only unnecessary-mut warnings, one Unix needless-return warning, and a CRLF-checkout mismatch in the generated-PowerShell fixture; all shared failures came from those four diagnostics. The Windows-only retirement finalizer and newline-independent anchor normalization, including an explicit CRLF hardener regression fixture, pass locally and are being pushed additively (agent: codex)
