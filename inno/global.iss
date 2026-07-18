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
#ifdef ND300_ROLLBACK_TEST
OutputBaseFilename=nd300-x86_64-pc-windows-msvc-setup-rollback-test
#else
OutputBaseFilename=nd300-x86_64-pc-windows-msvc-setup
#endif
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
; Show the PolyForm Noncommercial 1.0.0 LICENSE in the installer wizard
; (consistent with TR-300; the LICENSE file is committed at the repo root).
LicenseFile=..\LICENSE
SetupLogging=yes
; Never ask Restart Manager to terminate the updater that launched Setup. The
; PrepareToInstall hook below transactionally retires the running image first.
AppMutex=ND300_Running
CloseApplications=no
RestartApplications=no

[Tasks]
; Consolidation tasks — BOTH default-checked (Operator policy: one install at a
; time). Under /SILENT, default-checked tasks fire automatically, so the silent
; self-update path (`nd300 update` -> EXE installer /SILENT) runs both cleanups
; with NO /MERGETASKS suppression needed. The user can untick either in the
; interactive wizard.
Name: "cleancargo"; Description: "Remove an older Cargo-installed copy of nd300 (recommended — keeps one version on PATH)"; GroupDescription: "Consolidate installs:"
Name: "cleanotheredition"; Description: "Remove the other edition (Corporate per-user) if present (recommended — one edition at a time)"; GroupDescription: "Consolidate installs:"

[Languages]
Name: "english"; MessagesFile: "compiler:Default.isl"

[Files]
; Bundles both nd300.exe and speedqx.exe from target/release/. The CI workflow
; runs cargo build --release before invoking iscc so these paths are populated.
; A temp-only copy runs the hidden registered-channel takeover before files move.
Source: "..\target\release\{#MyAppExeName}"; DestName: "nd300-install-takeover.exe"; Flags: dontcopy
Source: "..\target\release\{#MyAppExeName}"; DestDir: "{app}\bin"; Flags: ignoreversion
Source: "..\target\release\{#MySecondExeName}"; DestDir: "{app}\bin"; Flags: ignoreversion

[Registry]
; Install-source marker. `nd300 update` reads HKCU\Software\ND300\InstallSource
; and picks the matching installer to download for in-place upgrades. Value must
; match the `exe-global` arm in src/actions/update.rs::read_install_source_marker().
Root: HKCU; Subkey: "Software\ND300"; ValueType: string; ValueName: "InstallSource"; ValueData: "exe-global"; Flags: uninsdeletevalue
Root: HKCU; Subkey: "Software\ND300"; Flags: uninsdeletekeyifempty

[Run]
; Always clean versioned images retired by PrepareToInstall. The new binary's
; allowlisted helper deletes immediately or schedules deletion after the old
; updater exits.
Filename: "{app}\bin\{#MyAppExeName}"; Parameters: "migrate-cleanup --quiet --retired-update"; Flags: runhidden waituntilterminated; StatusMsg: "Finalizing the in-place update..."
; Post-install consolidation. The deletion LOGIC lives in the binary
; (`nd300 migrate-cleanup`), which only ever removes nd300.exe/speedqx.exe, never
; cargo/rustup, never the .cargo\bin PATH entry, never the running install, and
; always exits 0 (cleanup is advisory — it must never fail the install).
;
; perMachine Global EXE runs ELEVATED. Inno has no reliable constant for the
; pre-elevation (invoking) user's profile: there is no {userprofile} constant,
; and {%USERPROFILE} / runasoriginaluser resolve to the admin account when a
; standard user supplies separate admin credentials. So we do NOT pass
; --user-profile here — migrate-cleanup falls back to the process environment
; (CARGO_HOME, then %USERPROFILE% / %LocalAppData%), which is correct in the
; common case (an admin elevating their own session) and FAILS SAFE otherwise
; (it finds no copy under the admin profile and reports a harmless no-op). The
; perMachine MSI resolves the right user via an Impersonate='yes' custom action.
;
; runhidden + waituntilterminated keeps the wizard clean and ordered; nowait is
; deliberately NOT used so cleanup finishes before Setup reports done.
Filename: "{app}\bin\{#MyAppExeName}"; Parameters: "migrate-cleanup --quiet --install-origin exe-global --cargo-copy"; Flags: runhidden waituntilterminated; Tasks: cleancargo; StatusMsg: "Removing older Cargo-installed copy..."
Filename: "{app}\bin\{#MyAppExeName}"; Parameters: "migrate-cleanup --quiet --install-origin exe-global --other-edition"; Flags: runhidden waituntilterminated; Tasks: cleanotheredition; StatusMsg: "Removing the other edition..."

[Code]
{
  PATH management — system PATH (HKLM) for the Global perMachine edition.
  Inno Setup's [Registry] section can't safely append-without-duplicates +
  reliably remove-on-uninstall, so we do it explicitly in [Code].
  The canonical pattern, adapted from the Inno Setup community knowledge base.
}
const
  EnvironmentKey = 'SYSTEM\CurrentControlSet\Control\Session Manager\Environment';

var
  RetiredNd300: Boolean;
  RetiredSpeedqx: Boolean;
  InstallSucceeded: Boolean;

function RetireBinary(CurrentPath, RetiredPath: string; var Retired: Boolean): Boolean;
begin
  Result := True;
  Retired := False;
  if not FileExists(CurrentPath) then exit;
  if FileExists(RetiredPath) and (not DeleteFile(RetiredPath)) then
  begin
    Result := False;
    exit;
  end;
  Retired := RenameFile(CurrentPath, RetiredPath);
  Result := Retired;
end;

procedure RestoreRetiredBinary(CurrentPath, RetiredPath: string; Retired: Boolean);
begin
  if Retired and FileExists(RetiredPath) then
  begin
    if FileExists(CurrentPath) then
      DeleteFile(CurrentPath);
    RenameFile(RetiredPath, CurrentPath);
  end;
end;

function RunChannelTakeover(Target: string): string;
var
  HelperPath: string;
  ExitCode: Integer;
begin
  Result := '';
  ExtractTemporaryFile('nd300-install-takeover.exe');
  HelperPath := ExpandConstant('{tmp}\nd300-install-takeover.exe');
  if not Exec(
    HelperPath,
    'install-takeover --target ' + Target + ' --scope all',
    '', SW_HIDE, ewWaitUntilTerminated, ExitCode) then
  begin
    Result := 'Could not start the ND-300 channel takeover helper.';
    exit;
  end;
  if ExitCode <> 0 then
    Result := 'Could not safely replace the previously registered ND-300 install. See https://reports.qubetx.com/nd300#install';
end;

function PrepareToInstall(var NeedsRestart: Boolean): String;
var
  Nd300Path, Nd300Retired, SpeedqxPath, SpeedqxRetired: string;
begin
  Result := RunChannelTakeover('exe-global');
  if Result <> '' then exit;
  Nd300Path := ExpandConstant('{app}\bin\{#MyAppExeName}');
  Nd300Retired := ExpandConstant('{app}\bin\nd300.update-old-{#MyAppVersion}.exe');
  SpeedqxPath := ExpandConstant('{app}\bin\{#MySecondExeName}');
  SpeedqxRetired := ExpandConstant('{app}\bin\speedqx.update-old-{#MyAppVersion}.exe');

  if not RetireBinary(Nd300Path, Nd300Retired, RetiredNd300) then
  begin
    Result := 'Could not prepare nd300.exe for an in-place update.';
    exit;
  end;
  if not RetireBinary(SpeedqxPath, SpeedqxRetired, RetiredSpeedqx) then
  begin
    RestoreRetiredBinary(Nd300Path, Nd300Retired, RetiredNd300);
    RetiredNd300 := False;
    Result := 'Could not prepare speedqx.exe for an in-place update.';
  end;
#ifdef ND300_ROLLBACK_TEST
  if Result = '' then
    Result := 'ND300 installer rollback test after running-image retirement.';
#endif
end;

procedure DeinitializeSetup;
begin
  if not InstallSucceeded then
  begin
    RestoreRetiredBinary(
      ExpandConstant('{app}\bin\{#MyAppExeName}'),
      ExpandConstant('{app}\bin\nd300.update-old-{#MyAppVersion}.exe'),
      RetiredNd300);
    RestoreRetiredBinary(
      ExpandConstant('{app}\bin\{#MySecondExeName}'),
      ExpandConstant('{app}\bin\speedqx.update-old-{#MyAppVersion}.exe'),
      RetiredSpeedqx);
  end;
end;

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
    EnvAddPath(ExpandConstant('{app}') + '\bin')
  else if CurStep = ssDone then
    InstallSucceeded := True;
end;

procedure CurUninstallStepChanged(CurUninstallStep: TUninstallStep);
begin
  if CurUninstallStep = usPostUninstall then
    EnvRemovePath(ExpandConstant('{app}') + '\bin');
end;
