; ND-300 — Corporate Edition installer (perUser, no admin required).
;
; Built by .github/workflows/windows-installers.yml after release.yml finishes
; publishing the GitHub Release. Companion to inno/global.iss (perMachine
; sibling). The MSI sibling lives at wix-corporate/corporate.wxs.
;
; Builds with Inno Setup 6 (`iscc` from JRSoftware) on a Windows runner. CI
; passes the version via `iscc /DMyAppVersion=3.1.0`.
;
; ND-300 ships TWO binaries (nd300.exe + speedqx.exe), so this installer
; packages BOTH (two [Files] lines). All four installers (MSI Global, MSI
; Corporate, EXE Global, EXE Corporate) target only TWO actual install paths —
; one per edition. The Corporate edition (both MSI and EXE) installs to
;     %LocalAppData%\Programs\nd300\bin\
; and modifies USER PATH (HKCU\Environment\Path), not system PATH. README
; documents "pick one format per edition" since coexistence creates duplicate
; Add/Remove Programs entries.
;
; LOCKSTEP: if the install path changes here, update
; src/actions/update.rs::classify_install_path() to match.

#ifndef MyAppVersion
  #define MyAppVersion "0.0.0-dev"
#endif

#define MyAppName "nd300"
#define MyAppFullName "nd300 (Corporate Edition)"
#define MyAppPublisher "Emmett S"
#define MyAppURL "https://github.com/QubeTX/qube-network-diagnostics"
#define MyAppExeName "nd300.exe"
#define MySecondExeName "speedqx.exe"

[Setup]
; AppId is the immutable identity of the Corporate EXE installer. Different GUID
; from both the Corporate MSI's UpgradeCode and the Global EXE's AppId so the
; four installer products are distinct to Windows.
AppId={{B6A0E3BD-BDD8-44A3-B524-C226B2A116A9}
AppName={#MyAppFullName}
AppVersion={#MyAppVersion}
AppVerName={#MyAppFullName} {#MyAppVersion}
AppPublisher={#MyAppPublisher}
AppPublisherURL={#MyAppURL}
AppSupportURL={#MyAppURL}
AppUpdatesURL={#MyAppURL}/releases
; perUser install location: %LocalAppData%\Programs\nd300 — same path as the
; Corporate MSI by design. {userpf} is Inno Setup's per-user "Program Files"
; equivalent and resolves to %LocalAppData%\Programs.
DefaultDirName={userpf}\{#MyAppName}
DefaultGroupName={#MyAppName}
DisableProgramGroupPage=yes
DisableDirPage=auto
; The core perUser switch: lowest privileges, no admin elevation, no UAC.
; PrivilegesRequiredOverridesAllowed= prevents Inno from offering the user the
; choice to elevate (we deliberately install per-user only).
PrivilegesRequired=lowest
PrivilegesRequiredOverridesAllowed=
ArchitecturesAllowed=x64
ArchitecturesInstallIn64BitMode=x64
OutputBaseFilename=nd300-x86_64-pc-windows-msvc-corporate-setup
OutputDir=Output
Compression=lzma
SolidCompression=yes
WizardStyle=modern
ChangesEnvironment=yes
; ARP display name. Matches the Corporate MSI's Product Name so the two installer
; formats show consistent labels.
UninstallDisplayName={#MyAppFullName}
; Show the PolyForm Noncommercial 1.0.0 LICENSE in the wizard (see inno/global.iss).
LicenseFile=..\LICENSE
SetupLogging=yes
; Cross-method consolidation (v3.2.0+) — see inno/global.iss for rationale.
AppMutex=ND300_Running
CloseApplications=yes

[Tasks]
; Both default-checked (one install at a time). Under /SILENT (the silent
; self-update path), default-checked tasks fire automatically — both cleanups run
; with no /MERGETASKS suppression needed. The user can untick either in the
; interactive wizard.
Name: "cleancargo"; Description: "Remove an older Cargo-installed copy of nd300 (recommended — keeps one version on PATH)"; GroupDescription: "Consolidate installs:"
Name: "cleanotheredition"; Description: "Remove the other edition (Global per-machine) if present (recommended — one edition at a time)"; GroupDescription: "Consolidate installs:"

[Languages]
Name: "english"; MessagesFile: "compiler:Default.isl"

[Files]
Source: "..\target\release\{#MyAppExeName}"; DestDir: "{app}\bin"; Flags: ignoreversion
Source: "..\target\release\{#MySecondExeName}"; DestDir: "{app}\bin"; Flags: ignoreversion

[Registry]
; Install-source marker. `nd300 update` reads HKCU\Software\ND300\InstallSource
; and picks the matching installer to download for in-place upgrades. Value must
; match the `exe-corporate` arm in
; src/actions/update.rs::read_install_source_marker().
Root: HKCU; Subkey: "Software\ND300"; ValueType: string; ValueName: "InstallSource"; ValueData: "exe-corporate"; Flags: uninsdeletevalue
Root: HKCU; Subkey: "Software\ND300"; Flags: uninsdeletekeyifempty

[Run]
; Post-install consolidation. Deletion LOGIC lives in the binary
; (`nd300 migrate-cleanup`) — it only ever removes nd300.exe/speedqx.exe, never
; cargo/rustup, never the .cargo\bin PATH entry, never the running install, and
; always exits 0 (advisory — never fails the install).
;
; perUser Corporate EXE runs AS THE USER (PrivilegesRequired=lowest, no UAC), so
; the child process's environment already points at the right .cargo and
; %LocalAppData% — no --user-profile needed. The "other edition" here is the
; Global perMachine copy in Program Files; a perUser process can't delete it, so
; migrate-cleanup reports "needs admin: <path>" and exits 0 (graceful skip).
Filename: "{app}\bin\{#MyAppExeName}"; Parameters: "migrate-cleanup --quiet --cargo-copy"; Flags: runhidden waituntilterminated; Tasks: cleancargo; StatusMsg: "Removing older Cargo-installed copy..."
Filename: "{app}\bin\{#MyAppExeName}"; Parameters: "migrate-cleanup --quiet --other-edition"; Flags: runhidden waituntilterminated; Tasks: cleanotheredition; StatusMsg: "Removing the other edition..."

[Code]
{
  PATH management — user PATH (HKCU\Environment\Path) for the Corporate perUser
  edition. Same canonical pattern as inno/global.iss but pointing at the
  HKCU\Environment key instead of HKLM\...\Session Manager\Environment.
}
const
  EnvironmentKey = 'Environment';

procedure EnvAddPath(Path: string);
var
  Paths: string;
begin
  if not RegQueryStringValue(HKEY_CURRENT_USER, EnvironmentKey, 'Path', Paths) then
    Paths := '';

  // Skip if already in PATH (case-insensitive substring match with ;-padding so
  // we don't match a prefix of a different directory).
  if Pos(';' + Uppercase(Path) + ';', ';' + Uppercase(Paths) + ';') > 0 then exit;

  if Length(Paths) > 0 then
    Paths := Paths + ';' + Path
  else
    Paths := Path;

  RegWriteExpandStringValue(HKEY_CURRENT_USER, EnvironmentKey, 'Path', Paths);
end;

procedure EnvRemovePath(Path: string);
var
  Paths: string;
  P: Integer;
begin
  if not RegQueryStringValue(HKEY_CURRENT_USER, EnvironmentKey, 'Path', Paths) then
    exit;

  P := Pos(';' + Uppercase(Path) + ';', ';' + Uppercase(Paths) + ';');
  if P = 0 then exit;

  if P = 1 then
    // First-entry case: consume the path plus its trailing `;` (see global.iss).
    //   Paths = "X;Y"  -> "Y"    (eats "X;")
    //   Paths = "X;"   -> ""     (eats "X;")
    //   Paths = "X"    -> ""     (eats "X", count clamps to remaining)
    Delete(Paths, 1, Length(Path) + 1)
  else
    // Middle/end entry: consume the leading `;` plus the path.
    //   Paths = "A;X;B" -> "A;B" (eats ";X")
    //   Paths = "A;X"   -> "A"   (eats ";X")
    Delete(Paths, P - 1, Length(Path) + 1);

  RegWriteExpandStringValue(HKEY_CURRENT_USER, EnvironmentKey, 'Path', Paths);
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
