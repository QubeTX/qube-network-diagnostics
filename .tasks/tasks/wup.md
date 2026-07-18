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
Active. Public v3.6.2 matrix run `29610274431` failed all five origins. Diagnostic run `29611079431` proved Cargo and cargo-dist could not overwrite the running Windows image; installer origins were terminated by Restart Manager/application shutdown before reporting. v3.6.3 applies a channel-preserving, exact-release preserve/replace/restore transaction. PR #21 exact head `9e50332` is cross-platform green; replacement candidate matrix `29623077460` has proved Cargo, Corporate managed channels, same-scope takeover, and opposite-scope refusal while isolating the remaining Global MSI failure to a malformed quoted commit-action path. The corrected production and rollback MSI packages compile locally and await exact hosted proof. No merge or tag occurs before the entire disposable Windows candidate matrix passes.

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
- 2026-07-17 19:08 — exact-head run `29622217262` passed all 321 Windows Rust tests and the hardener fixture's assertions, then revealed that an intentionally nonzero advisory cleanup leaked PowerShell's native exit code after printing `PASS`. The production shim now captures cleanup through `Start-Process` and regression-checks that a completed update exits successfully (agent: codex)
- 2026-07-17 19:20 — exact-head CI `29622370475` is fully green, while candidate matrix `29622370440` separated one passing Cargo lifecycle and three passing same-scope takeovers from three concrete harness/ownership defects plus a still-unexplained Global MSI 1603. Corporate managed-MSI scope now follows proven channel instead of ARP hive, Inno fault injection uses its documented fatal preinstall result, refusal cleanup uses the exact old MSI registration, and permanent verbose MSI evidence is enabled for the replacement run (agent: codex)
- 2026-07-17 19:34 — exact-head CI `29623077450` passed at `9e50332`; replacement matrix `29623077460` proved the Corporate and scope-classification fixes. Its first verbose Global MSI log identified the 1603 exactly: WiX quoted `[Bin]` and `[LegacyBin]`, whose trailing backslashes escaped the closing quotes and made `install-maintenance` exit 2. The action now constructs non-trailing `APPLICATIONFOLDER\bin` paths; production and rollback MSI builds both pass locally (agent: codex)
