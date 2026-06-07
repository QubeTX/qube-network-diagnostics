; ND-300 — Global Edition installer (perMachine, requires admin).
;
; Built by .github/workflows/windows-installers.yml after release.yml finishes
; publishing the GitHub Release. Companion to inno/corporate.iss (perUser
; sibling). The MSI sibling lives at wix/main.wxs.
;
; Builds with Inno Setup 6 (`iscc` from JRSoftware) on a Windows runner. CI
; passes the version via `iscc /DMyAppVersion=3.1.0` so the same script rebuilds
; at every release without editing.
;
; ND-300 ships TWO binaries (nd300.exe + speedqx.exe), so this installer
; packages BOTH (two [Files] lines). The MSI sibling and both EXE editions
; target the SAME install path per edition (Global → C:\Program Files\nd300\bin)
; and write the SAME registry marker key (HKCU\Software\ND300) — only the
; InstallSource value differs (msi-global vs exe-global). README documents "pick
; one format per edition" since coexistence creates duplicate Add/Remove
; Programs entries.
;
; LOCKSTEP: if the install path changes here, update
; src/actions/update.rs::classify_install_path() — that function matches on the
; install path to choose which installer to fetch during `nd300 update`.

#ifndef MyAppVersion
  #define MyAppVersion "0.0.0-dev"
#endif

#define MyAppName "nd300"
#define MyAppPublisher "Emmett S"
#define MyAppURL "https://github.com/QubeTX/qube-network-diagnostics"
#define MyAppExeName "nd300.exe"
#define MySecondExeName "speedqx.exe"

[Setup]
; AppId is the immutable identity of the Global EXE installer. Different from the
; MSI Global's UpgradeCode (Windows treats MSI products and Inno Setup products
; as separate even when they target the same install path) and different from the
; Corporate EXE's AppId.
AppId={{13F102E1-E08D-4C4E-ABA6-7D77604DFECD}
AppName={#MyAppName}
AppVersion={#MyAppVersion}
AppVerName={#MyAppName} {#MyAppVersion}
AppPublisher={#MyAppPublisher}
AppPublisherURL={#MyAppURL}
AppSupportURL={#MyAppURL}
AppUpdatesURL={#MyAppURL}/releases
; perMachine install: %ProgramFiles%\nd300 — same path as MSI Global by design.
DefaultDirName={commonpf}\{#MyAppName}
DefaultGroupName={#MyAppName}
; CLI tool — no start menu group, no desktop shortcut.
DisableProgramGroupPage=yes
DisableDirPage=auto
; Require admin (perMachine scope). Triggers UAC prompt at install start.
PrivilegesRequired=admin
PrivilegesRequiredOverridesAllowed=
ArchitecturesAllowed=x64
ArchitecturesInstallIn64BitMode=x64
OutputBaseFilename=nd300-x86_64-pc-windows-msvc-setup
OutputDir=Output
Compression=lzma
SolidCompression=yes
WizardStyle=modern
; Tell Windows we touched env vars so File Explorer broadcasts WM_SETTINGCHANGE.
; New cmd / PowerShell sessions then pick up the PATH addition without reboot.
ChangesEnvironment=yes
; ARP display name. Matches the MSI Global's Product Name so users see the same
; label regardless of which installer they used.
UninstallDisplayName={#MyAppName}
; ND-300 has no committed LICENSE file (SPDX PolyForm-Noncommercial-1.0.0 is
; declared in Cargo.toml; cargo-wix also sets license=false). So we deliberately
; omit LicenseFile here — referencing a missing file would fail the iscc build.
SetupLogging=yes

[Languages]
Name: "english"; MessagesFile: "compiler:Default.isl"

[Files]
; Bundles both nd300.exe and speedqx.exe from target/release/. The CI workflow
; runs cargo build --release before invoking iscc so these paths are populated.
Source: "..\target\release\{#MyAppExeName}"; DestDir: "{app}\bin"; Flags: ignoreversion
Source: "..\target\release\{#MySecondExeName}"; DestDir: "{app}\bin"; Flags: ignoreversion

[Registry]
; Install-source marker. `nd300 update` reads HKCU\Software\ND300\InstallSource
; and picks the matching installer to download for in-place upgrades. Value must
; match the `exe-global` arm in src/actions/update.rs::read_install_source_marker().
Root: HKCU; Subkey: "Software\ND300"; ValueType: string; ValueName: "InstallSource"; ValueData: "exe-global"; Flags: uninsdeletevalue
Root: HKCU; Subkey: "Software\ND300"; Flags: uninsdeletekeyifempty

[Code]
{
  PATH management — system PATH (HKLM) for the Global perMachine edition.
  Inno Setup's [Registry] section can't safely append-without-duplicates +
  reliably remove-on-uninstall, so we do it explicitly in [Code].
  The canonical pattern, adapted from the Inno Setup community knowledge base.
}
const
  EnvironmentKey = 'SYSTEM\CurrentControlSet\Control\Session Manager\Environment';

procedure EnvAddPath(Path: string);
var
  Paths: string;
begin
  if not RegQueryStringValue(HKEY_LOCAL_MACHINE, EnvironmentKey, 'Path', Paths) then
    Paths := '';

  // Skip if already in PATH (case-insensitive substring match with ;-padding so
  // we don't match a prefix of a different directory).
  if Pos(';' + Uppercase(Path) + ';', ';' + Uppercase(Paths) + ';') > 0 then exit;

  if Length(Paths) > 0 then
    Paths := Paths + ';' + Path
  else
    Paths := Path;

  RegWriteExpandStringValue(HKEY_LOCAL_MACHINE, EnvironmentKey, 'Path', Paths);
end;

procedure EnvRemovePath(Path: string);
var
  Paths: string;
  P: Integer;
begin
  if not RegQueryStringValue(HKEY_LOCAL_MACHINE, EnvironmentKey, 'Path', Paths) then
    exit;

  P := Pos(';' + Uppercase(Path) + ';', ';' + Uppercase(Paths) + ';');
  if P = 0 then exit;

  if P = 1 then
    // First-entry case: consume the path plus its trailing `;`. Using
    // Delete(Paths, P - 1, ...) here = Delete(Paths, 0, ...), undefined in
    // Pascal Script (a no-op in Inno's runtime), would strand the entry.
    //   Paths = "X;Y"  -> "Y"    (eats "X;")
    //   Paths = "X;"   -> ""     (eats "X;")
    //   Paths = "X"    -> ""     (eats "X", count clamps to remaining)
    Delete(Paths, 1, Length(Path) + 1)
  else
    // Middle/end entry: consume the leading `;` plus the path.
    //   Paths = "A;X;B" -> "A;B" (eats ";X")
    //   Paths = "A;X"   -> "A"   (eats ";X")
    Delete(Paths, P - 1, Length(Path) + 1);

  RegWriteExpandStringValue(HKEY_LOCAL_MACHINE, EnvironmentKey, 'Path', Paths);
end;

procedure CurStepChanged(CurStep: TSetupStep);
begin
  if CurStep = ssPostInstall then
    EnvAddPath(ExpandConstant('{app}') + '\bin');
end;

procedure CurUninstallStepChanged(CurUninstallStep: TUninstallStep);
begin
  if CurUninstallStep = usPostUninstall then
    EnvRemovePath(ExpandConstant('{app}') + '\bin');
end;
