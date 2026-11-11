# ND-300 4.0.2 security release and recovery evidence

## Scope and relevance

The operator requested investigation, conflict repair and deployment of relevant
fixes from security PRs #42–46. The [current-source review](security-pr-review-2026-11-10.md)
records the reachable conditions, primary references and limitations. #42 and
#43 remained relevant; #44 was partly superseded by existing permission/signing
controls; #45 supplied useful download-integrity hardening; #46's claimed
unsigned-script bypass was not demonstrated and its certificate/script pins
would reject legitimate packages. The replacement implementation also closes
the independently reproduced Bash startup/function and PowerShell module paths.

PR [#42](https://github.com/QubeTX/qube-network-diagnostics/pull/42) reconciled the
changes against main `d9f0d35be5767152d8d3c670a991537d0bc1c7cf` without rewriting
the original branch. Candidate `492b6822ea86f0b504b81ab51209e44ff9e1060a` merged
as `ceb10bf6e5818a36200c8292c78c3202160de864`. PRs #43–45 were closed with
supersession receipts, and #46 with the rejection evidence. Unrelated PR #38
result-insight work remains separate.

## Original 4.0.2 candidate validation

- Rust 1.97.0 formatting, strict all-feature Clippy, all-target tests (338
  library plus five CLI tests on Windows), optimized build, audit, dist plan,
  112-file package inspection and packaged-crate publication dry-run passed.
  The audit reports zero current vulnerabilities with no added exceptions.
- All four Windows installers built from the final pair. Release checksum,
  cargo-dist updater, downloaded-tool integrity and Mac parser fixtures passed;
  all 15 Apple signing-policy tests passed. Actionlint covered every workflow,
  ShellCheck covered every release shell script, and the secret scan passed
  using the available regex fallback.
- Harmless tests include positive exploit controls for inherited program lookup,
  Bash startup/functions and PowerShell module discovery. The real Linux
  `wpa_supplicant` parser test passed explicitly in CI (one test, zero failures),
  using an unsupported driver before network-device initialization. These tests
  do not claim live repair/reconnection qualification.
- Independent security, cross-platform, installer LOCKSTEP and automation reviews
  approved the final changes. The optional PowerShell 7 lookup regression was
  resolved through deliberate retirement, documented native-PowerShell policy
  and 48 focused updater tests. All five cargo-dist digests and the Inno digest
  matched publisher metadata; actual Windows acquisition ran successfully in
  an isolated tool directory.

All five exact-candidate workflow gates passed:

| Gate | Run |
|---|---|
| Cross-platform CI | [42968217807](https://github.com/QubeTX/qube-network-diagnostics/actions/runs/42968217807) |
| Windows origins, fresh takeover and safe refusal | [42968217772](https://github.com/QubeTX/qube-network-diagnostics/actions/runs/42968217772) |
| Native unsigned Mac PR builds | [42968217743](https://github.com/QubeTX/qube-network-diagnostics/actions/runs/42968217743) |
| Complete measurement parity | [42968217736](https://github.com/QubeTX/qube-network-diagnostics/actions/runs/42968217736) |
| Release plan | [42968217846](https://github.com/QubeTX/qube-network-diagnostics/actions/runs/42968217846) |

The administrator publisher PR route was used as required by ruleset 24618672.
Separate integrity ruleset 24618673 has no bypass actors; its exact-head
`CI required` check passed before merge. No protection was weakened.
Read-only configuration verification on November 11 found zero repository-scope
Apple secrets, seven Apple secrets in `apple-signing`, and only the selected
`main` branch / `v*` tag policies. Secret values were not read or printed.

## Initial 4.0.2 qualification and incomplete publication

Exact-main [CI 42969643638](https://github.com/QubeTX/qube-network-diagnostics/actions/runs/42969643638)
and [measurement parity 42969643665](https://github.com/QubeTX/qube-network-diagnostics/actions/runs/42969643665)
passed at `ceb10bf6e5818a36200c8292c78c3202160de864`. The
[crate publisher 42970625622](https://github.com/QubeTX/qube-network-diagnostics/actions/runs/42970625622)
succeeded. Independent public download confirms nd300 4.0.2 is unyanked,
SHA-256 `dfa89c64561d145b89885a95ba4bec37440aac90a1c5c29718479fab36388c73`,
and its embedded `.cargo_vcs_info.json` names the exact main commit. A fresh
isolated `cargo +1.97.0 install nd300 --version 4.0.2 --locked --force` from the
public registry succeeded; both installed commands report 4.0.2. The active
installation and PATH were not changed.

[Signed Apple qualification 42969643742](https://github.com/QubeTX/qube-network-diagnostics/actions/runs/42969643742) passed in full on attempt 2, including both native lifecycles. The exact-source qualification helper confirmed all nine required jobs succeeded in that same attempt. Attempt 1 passed both native archives and both Installer identity preflights. Apple accepted the PKG and DMG notarizations; later DMG ticket validation failed with Apple CloudKit `NSURLErrorDomain -1005` (network connection lost), exit 68. This is a transient service failure, so source/version remain unchanged. The complete workflow was rerun to keep every qualification job in one successful attempt; no gate was bypassed. The previously unused `v4.0.2` tag was pushed once at the exact qualified main commit. The administrator publisher role was permitted to create it; the separate rules still forbid tag updates/deletion. [Release 42972625036](https://github.com/QubeTX/qube-network-diagnostics/actions/runs/42972625036) failed after publishing the base archives and Windows installers (28 assets). The four Mac package assets were not published. The tag and published assets remain unchanged. The `latest` designation was restored to the complete v4.0.1 release (32 assets; successful run 33954529791) during forward correction, so versionless Mac downloads continue to resolve. This changes release metadata only.

The release's Intel archive public check initially failed on fresh-runner Gatekeeper
`source=Unnotarized Developer ID` (job 130367380737). The download checksum
and code signature passed. Build job 130364467112 records Apple notarization
Accepted (submission `3d7e056e-826b-4009-b665-65bb9c55bc8d`) and uncached
Gatekeeper acceptance of both exact binaries before publication. This suggests
inconsistent online ticket availability; it is not proof that the public trust
gate passed. A read-only retry against the unchanged public bytes passed in attempt 2, job 130370511567. The initial response was transient; this did not resolve the separate missing-credentials failure.

## Withdrawn 4.0.3 preparation (historical validation)

Both reusable Mac credential preflights failed with empty Installer secret values
in the 4.0.2 tag run. The standalone main qualification had worked. The caller
omitted a named secret interface, which current [GitHub documentation](https://docs.github.com/en/actions/how-tos/reuse-automations/reuse-workflows)
requires even for secrets stored only in the called job's environment. The
replacement declares exactly seven required credential names and forwards only
those names. The protected environment, source/qualification gates and prohibition
on broad inheritance remain in force. No secret value or repository-scope secret
was added. The existing static test had incorrectly required absence of a caller secrets map;
the regression now verifies the allowlist and protected signing jobs separately.

The initially prepared 4.0.3 version was later withdrawn before publication in favor of same-version delivery recovery. Its local Rust 1.97 formatting,
strict all-feature Clippy, 338 library/five CLI tests, optimized build, all four
Windows installer builds, zero-vulnerability audit, dist plan, full Actionlint/
ShellCheck and the regex secret scan passed. All 16 signing-policy tests passed,
including the newly demonstrated regression. Independent workflow/security
review approved the exact seven-name interface. Package inspection (112 files) and the committed-crate publication dry run passed.
All five exact-candidate workflows passed at `f6a9ec49adf06398c2dfd6316637dda26a575291`:
CI 42975441097, Windows matrix 42975441102, native Mac PR builds 42975441128,
measurement parity 42975441195 and release plan 42975441197. PR #47 merged as
`1364ccc95fb99cb287da637c4601114154715dc0` through the documented administrator
publisher PR route after its mandatory CI check passed. Exact-main qualification,
crate publication, 32 public assets, native Apple trust/compatibility and homepage
delivery remain open. No 4.0.2 gate is waived.

## Earlier homepage preview (superseded below)

Homepage [PR #23](https://github.com/QubeTX/qube-machine-report-homepage/pull/23)
previously used candidate `57fe25099a2b9f7bb00cd219015a5fe9669010c9` for 4.0.3. Local lint/build,
secret scan, independent review and the Vercel preview passed. Chrome inspection of the final preview
confirmed the page rendered and the Mac install selector showed the direct PKG
link. No application-origin warning/error was captured; one unrelated browser
extension passkey-injection connection error was observed. The live release lookup correctly remained
4.0.1 before publication. The final preview deployment is `dpl_7P8V1TMHm5D2duLfLEqTjLdxHihq` and READY at the candidate SHA. Production delivery is pending public CLI verification.

## Operator pause and reassessment — November 11

The operator explicitly requested a pause to investigate the full release path
and avoid repeated version bumps. 4.0.3 is merged but unpublished: no tag exists,
and crates.io returns 404 for that version. Main CI 42976850192 was cancelled to
prevent automatic publication; crate publisher 42977317758 skipped. The separate
non-publishing Mac qualification 42976850225 was also cancelled. No code/test
failure is inferred from those deliberate cancellations.

The existing immutable v4.0.2 workflow has a standalone tag-bound repair dispatch
that selects environment credentials directly. This path may finish the four
missing Mac assets without replacing any of the 28 existing files. Its exact
source checks, qualification, 32-asset verification and native compatibility
requirements still need to be evaluated before execution. No repair, new tag,
crate publication, or further main merge is being attempted during the pause.

The automatic crate publisher was then temporarily disabled (`disabled_manually`)
to prevent an unrelated later main merge from publishing the prepared 4.0.3
version during the operator's pause. Re-enable it deliberately when publication
is resumed. The read-only recovery audit confirms that the v4.0.2 standalone
repair can use environment credentials directly and add only the four absent
assets. Its compatibility bridge requires latest=v4.0.2, so promotion must be
coordinated after the complete asset set is ready. The original failed root run
will remain failed; any recovery must preserve distinct successful repair receipts.

## Resumed recovery without another version — November 11

The operator resumed testing, publication and deployment, specifically following
the investigation into unnecessary version churn. The unpublished 4.0.3 bump is
withdrawn while retaining the explicit seven-secret correction. A manual main-only
caller will exercise the actual reusable credential boundary without publishing.
After that succeeds, the existing v4.0.2 standalone repair will add only the four
absent Mac assets from qualified source `ceb10bf`; no existing asset/tag will be
replaced. Latest promotion occurs only after all 32 assets are present and verified,
then both compatibility bridges must pass. The failed original root run remains
part of the record. Crate publishing stays disabled until main returns to the
already-published version and final checks establish safe resumption.

The nonpublishing caller passed 19 policy/contract tests, all-workflow Actionlint,
independent workflow/security review, and the full regex secret scan. Rust 1.97
formatting, strict all-feature Clippy and all-target tests (338 library and five
CLI tests) passed after restoring the already-published version metadata.
Hosted preflight and immutable-tag repair remain pending.
