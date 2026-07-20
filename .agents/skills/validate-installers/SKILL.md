---
name: validate-installers
description: This skill should be used when the user asks to "validate installers", "build the installers locally", "check the Windows installers", "test the MSI/EXE installers", "run candle/light", "run iscc", or wants to catch ND-300 installer-build bugs (WiX CNDL0104, Inno iscc errors) BEFORE a tag-push release. Builds all four ND-300 Windows installers on the local machine and reports exit codes.
disable-model-invocation: true
allowed-tools: Read, Bash, Glob
---

# validate-installers

Build all four ND-300 Windows installers locally and report their exit codes,
reproducing what `.github/workflows/windows-installers.yml` does in CI. Run this
**before** a tag-push release so installer-build bugs surface locally instead of
at release time.

## Why this exists

ND-300 ships four first-class Windows installers, each packaging **both**
`nd300.exe` and `speedqx.exe`:

| Edition | Format | Source file | light flags |
|---|---|---|---|
| Global | MSI | `wix/main.wxs` | (none extra) |
| Corporate | MSI | `wix-corporate/corporate.wxs` | `-sice:ICE38 -sice:ICE64 -sice:ICE91` |
| Global | EXE | `inno/global.iss` | n/a (Inno) |
| Corporate | EXE | `inno/corporate.iss` | n/a (Inno) |

The Global MSI is the cargo-dist base asset; the other three (plus `.sha256`
sidecars) are built by `windows-installers.yml`, which the tag-triggered Release
workflow calls directly through `workflow_call` with the validated tag and
source SHA after publishing the 22 base assets. So an installer-build bug is not
caught by the normal `cargo build` / `cargo publish` gate; it only blows up
post-tag, when the release is already half-shipped — which is exactly why this
skill validates them locally first.

History this prevents:
- **v3.2.0** broke because a `--` (double hyphen) appeared inside an XML comment
  in `wix/main.wxs` — WiX 3's `candle` rejects this with **CNDL0104**, and
  cargo-dist hides candle's stderr, so it only failed at release (fixed by
  e4a5060). The repo-local `wix_comment_guard.py` PreToolUse hook now blocks
  that at edit time; this skill is the belt-and-suspenders full build.
- **v3.2.1** broke because `inno/global.iss` used an invalid Inno constant
  (`{userprofile}`), an `iscc`-only error invisible to `cargo build` (fixed by
  ede79d8).

## How to run

The bundled PowerShell script does everything. From the repo root:

```powershell
pwsh -File .agents/skills/validate-installers/scripts/validate-installers.ps1
```

It will:

1. **Ensure WiX 3.11** (`candle.exe` / `light.exe`). Checks `$env:WIX\bin`, then
   PATH, then a local cache; if missing, downloads
   `wix311-binaries.zip` from the WiX Toolset GitHub release into
   `%LOCALAPPDATA%\nd300-installer-tools\` and extracts it.
2. **Ensure Inno Setup 6** (`iscc.exe`). Checks PATH and the standard install
   dirs; if missing, downloads and installs it
   **`/CURRENTUSER /VERYSILENT /SUPPRESSMSGBOXES /NORESTART`** (no admin needed).
3. **`cargo build --release`** (unless `-SkipBuild`) → `target\release\nd300.exe`
   + `speedqx.exe`. Both must exist or the script aborts (every installer
   packages both binaries).
4. **Build all four installers** with the exact CI mechanics:
   - Global MSI: `candle -arch x64 "-dVersion=X" "-dCargoTargetBinDir=target\release" -ext WixUIExtension` then `light -ext WixUIExtension`.
   - Corporate MSI: same `candle`, then `light -sice:ICE38 -sice:ICE64 -sice:ICE91 -ext WixUIExtension` (the ICE suppressions are required — perUser MSI convention violations are cosmetic here).
   - Global EXE: `iscc "/DMyAppVersion=X" inno\global.iss`.
   - Corporate EXE: `iscc "/DMyAppVersion=X" inno\corporate.iss`.
5. **Print a summary table of exit codes** and exit non-zero if any installer
   failed.

Version defaults to `Cargo.toml`'s `[package] version`; override with
`-Version 3.4.0`. Skip the Rust build with `-SkipBuild` if `target\release`
is already current.

## The PowerShell arg-quoting gotcha (do not drop the quotes)

Every `-d...` (candle) and `/D...` (iscc) argument is passed as a **single
quoted string** — `"-dVersion=$Version"`, `"/DMyAppVersion=$Version"`. Without
the quotes, PowerShell's tokenizer splits `-dVersion=3.3.0` into `-dVersion=3`
followed by `.3.0` (the dots break tokenization), so `candle`/`iscc` treat the
stray `.3.0` as a source filename and fail (CNDL0103 / iscc error). If editing
the invocations, keep them quoted.

## Interpreting results

- **All four `exit=0`** → installers are buildable; safe to proceed with the
  release. Outputs land in `target\wix\*.msi` and `inno\Output\*setup.exe`.
- **Any `FAIL`** → do NOT tag-push. Read the candle/light/iscc stderr printed
  above the summary. Common culprits: `CNDL0104` (`--` in an XML comment body),
  `CNDL0103` (dropped arg quotes), invalid Inno constant, a missing
  `target\release` binary, or a broken `[Files]`/component reference.

## Lockstep reminder

This skill builds the installers but does not verify the LOCKSTEP contract
(install paths + `HKCU\Software\ND300\InstallSource` markers staying in sync with
`src/actions/update.rs`). For that review, use the `installer-lockstep-reviewer`
subagent.

## Bundled resources

- **`scripts/validate-installers.ps1`** — the build-all-four driver described
  above. Cross-platform PowerShell (`pwsh`); Windows-only in practice because
  candle/light/iscc are Windows tools.
