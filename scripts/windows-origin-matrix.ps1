<#
.SYNOPSIS
  Exercise one ND-300 Windows install origin on a disposable GitHub runner.

.DESCRIPTION
  Installs a published baseline, records marker/ARP/PATH/binary ownership,
  upgrades either to locally built candidate artifacts or through `nd300
  update`, exercises dry-run migration, and verifies owner-aware uninstall plus
  the surviving-Cargo stale-marker case. This script is intentionally mutating;
  run it only on disposable Windows VMs.
#>
[CmdletBinding()]
param(
    [Parameter(Mandatory)]
    [ValidateSet('msi-global', 'msi-corporate', 'exe-global', 'exe-corporate', 'cargo')]
    [string]$Origin,

    [Parameter(Mandatory)]
    [ValidateSet('candidate', 'public')]
    [string]$Mode,

    [ValidateSet('lifecycle', 'takeover', 'refusal')]
    [string]$Scenario = 'lifecycle',

    [ValidateSet('msi-global', 'msi-corporate', 'exe-global', 'exe-corporate', 'cargo')]
    [string]$BaselineOrigin,

    [switch]$LegacyGlobalBaseline,

    [Parameter(Mandatory)]
    [string]$BaselineVersion,

    [Parameter(Mandatory)]
    [string]$ExpectedVersion,

    [Parameter(Mandatory)]
    [string]$RepoRoot,

    [string]$CandidateDir,

    [string]$Repository = 'QubeTX/qube-network-diagnostics',

    [string]$EvidenceDir = (Join-Path $env:RUNNER_TEMP 'nd300-origin-evidence')
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'
$script:InitialProcessPath = $env:Path

# CreateProcessW with CREATE_SUSPENDED maps the exact baseline nd300.exe into a
# live process without allowing the CLI to exit. This reproduces the Windows
# running-image lock deterministically while keeping the test non-interactive.
Add-Type -TypeDefinition @'
using System;
using System.ComponentModel;
using System.IO;
using System.Runtime.InteropServices;
using System.Text;

public sealed class Nd300SuspendedImage : IDisposable
{
    private const uint CreateSuspended = 0x00000004;
    private const uint CreateNoWindow = 0x08000000;
    private const uint StillActive = 259;

    [StructLayout(LayoutKind.Sequential, CharSet = CharSet.Unicode)]
    private struct StartupInfo
    {
        public int cb;
        public string lpReserved;
        public string lpDesktop;
        public string lpTitle;
        public uint dwX;
        public uint dwY;
        public uint dwXSize;
        public uint dwYSize;
        public uint dwXCountChars;
        public uint dwYCountChars;
        public uint dwFillAttribute;
        public uint dwFlags;
        public short wShowWindow;
        public short cbReserved2;
        public IntPtr lpReserved2;
        public IntPtr hStdInput;
        public IntPtr hStdOutput;
        public IntPtr hStdError;
    }

    [StructLayout(LayoutKind.Sequential)]
    private struct ProcessInformation
    {
        public IntPtr hProcess;
        public IntPtr hThread;
        public uint dwProcessId;
        public uint dwThreadId;
    }

    [DllImport("kernel32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
    [return: MarshalAs(UnmanagedType.Bool)]
    private static extern bool CreateProcessW(
        string applicationName,
        StringBuilder commandLine,
        IntPtr processAttributes,
        IntPtr threadAttributes,
        bool inheritHandles,
        uint creationFlags,
        IntPtr environment,
        string currentDirectory,
        ref StartupInfo startupInfo,
        out ProcessInformation processInformation);

    [DllImport("kernel32.dll", SetLastError = true)]
    [return: MarshalAs(UnmanagedType.Bool)]
    private static extern bool GetExitCodeProcess(IntPtr process, out uint exitCode);

    [DllImport("kernel32.dll", SetLastError = true)]
    [return: MarshalAs(UnmanagedType.Bool)]
    private static extern bool TerminateProcess(IntPtr process, uint exitCode);

    [DllImport("kernel32.dll", SetLastError = true)]
    private static extern uint WaitForSingleObject(IntPtr handle, uint milliseconds);

    [DllImport("kernel32.dll", SetLastError = true)]
    [return: MarshalAs(UnmanagedType.Bool)]
    private static extern bool CloseHandle(IntPtr handle);

    private IntPtr processHandle;
    private IntPtr threadHandle;
    public uint ProcessId { get; private set; }

    private Nd300SuspendedImage(ProcessInformation info)
    {
        processHandle = info.hProcess;
        threadHandle = info.hThread;
        ProcessId = info.dwProcessId;
    }

    public static Nd300SuspendedImage Start(string executable)
    {
        var startup = new StartupInfo { cb = Marshal.SizeOf<StartupInfo>() };
        var command = new StringBuilder("\"" + executable + "\" --version");
        ProcessInformation info;
        if (!CreateProcessW(
                executable,
                command,
                IntPtr.Zero,
                IntPtr.Zero,
                false,
                CreateSuspended | CreateNoWindow,
                IntPtr.Zero,
                Path.GetDirectoryName(executable),
                ref startup,
                out info))
        {
            throw new Win32Exception(Marshal.GetLastWin32Error(), "CreateProcessW failed");
        }
        return new Nd300SuspendedImage(info);
    }

    public bool IsAlive
    {
        get
        {
            uint exitCode;
            return processHandle != IntPtr.Zero
                && GetExitCodeProcess(processHandle, out exitCode)
                && exitCode == StillActive;
        }
    }

    public void Stop()
    {
        if (processHandle != IntPtr.Zero && IsAlive)
        {
            if (!TerminateProcess(processHandle, 0))
            {
                throw new Win32Exception(Marshal.GetLastWin32Error(), "TerminateProcess failed");
            }
            WaitForSingleObject(processHandle, 5000);
        }
        Dispose();
    }

    public void Dispose()
    {
        if (threadHandle != IntPtr.Zero)
        {
            CloseHandle(threadHandle);
            threadHandle = IntPtr.Zero;
        }
        if (processHandle != IntPtr.Zero)
        {
            CloseHandle(processHandle);
            processHandle = IntPtr.Zero;
        }
        GC.SuppressFinalize(this);
    }
}
'@

function Invoke-Checked {
    param(
        [Parameter(Mandatory)][string]$FilePath,
        [Parameter(Mandatory)][string[]]$ArgumentList,
        [Parameter(Mandatory)][string]$Label
    )

    Write-Host "`n==> $Label"
    # PowerShell can return immediately for Windows GUI-subsystem executables
    # such as msiexec and Inno Setup, leaving $LASTEXITCODE stale while the
    # installer is still writing files. Use a process handle and wait explicitly;
    # ArgumentList also preserves paths without shell re-parsing.
    $startInfo = [System.Diagnostics.ProcessStartInfo]::new()
    $startInfo.FileName = $FilePath
    $startInfo.UseShellExecute = $false
    $startInfo.WorkingDirectory = (Get-Location).Path
    foreach ($argument in $ArgumentList) {
        [void]$startInfo.ArgumentList.Add($argument)
    }
    try {
        $process = [System.Diagnostics.Process]::Start($startInfo)
    } catch {
        throw "$Label could not start $FilePath`: $($_.Exception.Message)"
    }
    if (-not $process) { throw "$Label could not start $FilePath" }
    $process.WaitForExit()
    $code = $process.ExitCode
    if ($code -ne 0) {
        throw "$Label failed with exit code $code"
    }
}

function Invoke-Captured {
    param(
        [Parameter(Mandatory)][string]$FilePath,
        [Parameter(Mandatory)][string[]]$ArgumentList,
        [Parameter(Mandatory)][string]$Label
    )

    # GitHub's PowerShell runner promotes non-zero native exits to terminating
    # errors. A direct invocation can therefore abort before $LASTEXITCODE or
    # captured diagnostics are inspected. Use Process directly so every failure
    # retains stdout, stderr, and its real exit code in the job log.
    $startInfo = [System.Diagnostics.ProcessStartInfo]::new()
    $startInfo.FileName = $FilePath
    $startInfo.UseShellExecute = $false
    $startInfo.RedirectStandardOutput = $true
    $startInfo.RedirectStandardError = $true
    $startInfo.WorkingDirectory = (Get-Location).Path
    foreach ($argument in $ArgumentList) {
        [void]$startInfo.ArgumentList.Add($argument)
    }
    try {
        $process = [System.Diagnostics.Process]::Start($startInfo)
    } catch {
        throw "$Label could not start $FilePath`: $($_.Exception.Message)"
    }
    if (-not $process) { throw "$Label could not start $FilePath" }

    $stdoutTask = $process.StandardOutput.ReadToEndAsync()
    $stderrTask = $process.StandardError.ReadToEndAsync()
    $process.WaitForExit()
    $stdout = $stdoutTask.GetAwaiter().GetResult().Trim()
    $stderr = $stderrTask.GetAwaiter().GetResult().Trim()
    if ($stdout) { Write-Host $stdout }
    if ($stderr) { Write-Host $stderr }
    Write-Host "$Label exit=$($process.ExitCode) stdout_chars=$($stdout.Length) stderr_chars=$($stderr.Length)"

    return [pscustomobject]@{
        ExitCode = $process.ExitCode
        StdOut = $stdout
        StdErr = $stderr
    }
}

function Normalize-PathText {
    param([Parameter(Mandatory)][string]$Path)
    return $Path.Trim().TrimEnd([char[]]@('\', '/')).ToLowerInvariant()
}

function Test-PathListContains {
    param([AllowNull()][string]$Value, [Parameter(Mandatory)][string]$Target)
    if (-not $Value) { return $false }
    $expected = Normalize-PathText $Target
    foreach ($entry in ($Value -split ';')) {
        if ($entry.Trim() -and (Normalize-PathText $entry) -eq $expected) {
            return $true
        }
    }
    return $false
}

function Get-CargoBin {
    if ($env:CARGO_HOME) { return Join-Path $env:CARGO_HOME 'bin' }
    return Join-Path $env:USERPROFILE '.cargo\bin'
}

function Get-InstallBin {
    param([string]$ForOrigin)
    if (-not $ForOrigin) { $ForOrigin = $Origin }
    switch ($ForOrigin) {
        'msi-global' { return Join-Path $env:ProgramFiles 'nd300\bin' }
        'exe-global' { return Join-Path $env:ProgramFiles 'nd300\bin' }
        'msi-corporate' { return Join-Path $env:LOCALAPPDATA 'Programs\nd300\bin' }
        'exe-corporate' { return Join-Path $env:LOCALAPPDATA 'Programs\nd300\bin' }
        'cargo' { return Get-CargoBin }
    }
}

function Get-OriginMarker {
    param([string]$ForOrigin)
    if (-not $ForOrigin) { $ForOrigin = $Origin }
    if ($ForOrigin -eq 'cargo') { return $null }
    return $ForOrigin
}

function Get-Marker {
    try {
        return Get-ItemPropertyValue -LiteralPath 'HKCU:\Software\ND300' -Name InstallSource -ErrorAction Stop
    } catch {
        return $null
    }
}

function Get-PropertyValue {
    param(
        [Parameter(Mandatory)][object]$InputObject,
        [Parameter(Mandatory)][string]$Name
    )
    $property = $InputObject.PSObject.Properties[$Name]
    if ($property) { return $property.Value }
    return $null
}

function Get-ArpRecords {
    $roots = @(
        'HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\*',
        'HKLM:\Software\Microsoft\Windows\CurrentVersion\Uninstall\*',
        'HKLM:\Software\WOW6432Node\Microsoft\Windows\CurrentVersion\Uninstall\*'
    )
    return @(
        Get-ItemProperty -Path $roots -ErrorAction SilentlyContinue |
            Where-Object {
                (Get-PropertyValue -InputObject $_ -Name 'DisplayName') -in
                    @('nd300', 'nd-300', 'nd300 (Corporate Edition)')
            }
    )
}

function Get-InstallFingerprint {
    param([string]$InstallBin)
    if (-not $InstallBin) { $InstallBin = Get-InstallBin }
    $bin = $InstallBin
    $nd300 = Join-Path $bin 'nd300.exe'
    $speedqx = Join-Path $bin 'speedqx.exe'
    Refresh-ProcessPath
    $resolvedNd300 = Get-Command nd300.exe -ErrorAction SilentlyContinue
    $resolvedSpeedqx = Get-Command speedqx.exe -ErrorAction SilentlyContinue
    $arp = @(
        Get-ArpRecords |
            Sort-Object PSPath |
            ForEach-Object {
                [ordered]@{
                    key = Get-PropertyValue -InputObject $_ -Name 'PSChildName'
                    registry_path = Get-PropertyValue -InputObject $_ -Name 'PSPath'
                    display_name = Get-PropertyValue -InputObject $_ -Name 'DisplayName'
                    display_version = Get-PropertyValue -InputObject $_ -Name 'DisplayVersion'
                    install_location = Get-PropertyValue -InputObject $_ -Name 'InstallLocation'
                    windows_installer = Get-PropertyValue -InputObject $_ -Name 'WindowsInstaller'
                    uninstall_string = Get-PropertyValue -InputObject $_ -Name 'UninstallString'
                }
            }
    )
    return [ordered]@{
        marker = Get-Marker
        arp = $arp
        machine_path = [Environment]::GetEnvironmentVariable('Path', 'Machine')
        user_path = [Environment]::GetEnvironmentVariable('Path', 'User')
        nd300_path = $nd300
        nd300_sha256 = (Get-FileHash -LiteralPath $nd300 -Algorithm SHA256).Hash.ToLowerInvariant()
        speedqx_path = $speedqx
        speedqx_sha256 = (Get-FileHash -LiteralPath $speedqx -Algorithm SHA256).Hash.ToLowerInvariant()
        resolved_nd300 = if ($resolvedNd300) { $resolvedNd300.Source } else { $null }
        resolved_speedqx = if ($resolvedSpeedqx) { $resolvedSpeedqx.Source } else { $null }
    }
}

function Assert-FingerprintEqual {
    param(
        [Parameter(Mandatory)][object]$Expected,
        [Parameter(Mandatory)][object]$Actual,
        [Parameter(Mandatory)][string]$Label
    )
    $expectedJson = $Expected | ConvertTo-Json -Depth 8 -Compress
    $actualJson = $Actual | ConvertTo-Json -Depth 8 -Compress
    if ($expectedJson -cne $actualJson) {
        Write-Host "Expected fingerprint: $expectedJson"
        Write-Host "Actual fingerprint:   $actualJson"
        throw "$Label did not restore the exact binary, marker, ARP, and PATH ownership state"
    }
}

function Refresh-ProcessPath {
    $machine = [Environment]::GetEnvironmentVariable('Path', 'Machine')
    $user = [Environment]::GetEnvironmentVariable('Path', 'User')
    $env:Path = @($machine, $user, $script:InitialProcessPath) -join ';'
}

function Assert-Version {
    param([Parameter(Mandatory)][string]$Executable, [Parameter(Mandatory)][string]$Version)
    if (-not (Test-Path -LiteralPath $Executable -PathType Leaf)) {
        throw "Expected executable is missing: $Executable"
    }
    $result = Invoke-Captured $Executable @('--version') "Read $Executable version"
    if ($result.ExitCode -ne 0 -or $result.StdOut -notmatch "(?<![0-9])$([regex]::Escape($Version))(?![0-9])") {
        throw "$Executable did not report expected version $Version (exit=$($result.ExitCode), output=$($result.StdOut))"
    }
}

function Write-Snapshot {
    param(
        [Parameter(Mandatory)][string]$Label,
        [string]$InstallBin
    )
    New-Item -ItemType Directory -Force -Path $EvidenceDir | Out-Null
    Refresh-ProcessPath
    if (-not $InstallBin) { $InstallBin = Get-InstallBin }
    $bin = $InstallBin
    $nd300 = Join-Path $bin 'nd300.exe'
    $speedqx = Join-Path $bin 'speedqx.exe'
    $resolved = Get-Command nd300.exe -ErrorAction SilentlyContinue
    $arp = @(
        Get-ArpRecords | ForEach-Object {
            [ordered]@{
                key = Get-PropertyValue -InputObject $_ -Name 'PSChildName'
                registry_path = Get-PropertyValue -InputObject $_ -Name 'PSPath'
                display_name = Get-PropertyValue -InputObject $_ -Name 'DisplayName'
                display_version = Get-PropertyValue -InputObject $_ -Name 'DisplayVersion'
                install_location = Get-PropertyValue -InputObject $_ -Name 'InstallLocation'
                windows_installer = Get-PropertyValue -InputObject $_ -Name 'WindowsInstaller'
            }
        }
    )
    $machinePath = [Environment]::GetEnvironmentVariable('Path', 'Machine')
    $userPath = [Environment]::GetEnvironmentVariable('Path', 'User')
    $snapshot = [ordered]@{
        label = $Label
        origin = $Origin
        baseline_origin = $BaselineOrigin
        scenario = $Scenario
        legacy_global_baseline = [bool]$LegacyGlobalBaseline
        mode = $Mode
        baseline_version = $BaselineVersion
        expected_version = $ExpectedVersion
        marker = Get-Marker
        arp = $arp
        install_bin = $bin
        nd300_exists = Test-Path -LiteralPath $nd300 -PathType Leaf
        speedqx_exists = Test-Path -LiteralPath $speedqx -PathType Leaf
        nd300_sha256 = if (Test-Path -LiteralPath $nd300 -PathType Leaf) { (Get-FileHash -LiteralPath $nd300 -Algorithm SHA256).Hash.ToLowerInvariant() } else { $null }
        speedqx_sha256 = if (Test-Path -LiteralPath $speedqx -PathType Leaf) { (Get-FileHash -LiteralPath $speedqx -Algorithm SHA256).Hash.ToLowerInvariant() } else { $null }
        resolved_nd300 = if ($resolved) { $resolved.Source } else { $null }
        machine_path_has_install_bin = Test-PathListContains $machinePath $bin
        user_path_has_install_bin = Test-PathListContains $userPath $bin
    }
    $path = Join-Path $EvidenceDir "$Origin-$Label.json"
    $snapshot | ConvertTo-Json -Depth 8 | Set-Content -LiteralPath $path -Encoding utf8NoBOM
    Write-Host "Evidence: $path"
}

function Assert-InstallState {
    param(
        [Parameter(Mandatory)][string]$Version,
        [string]$ForOrigin,
        [string]$InstallBin,
        [switch]$ExpectNoMarker
    )
    if (-not $ForOrigin) { $ForOrigin = $Origin }
    if (-not $InstallBin) { $InstallBin = Get-InstallBin $ForOrigin }
    $bin = $InstallBin
    $nd300 = Join-Path $bin 'nd300.exe'
    $speedqx = Join-Path $bin 'speedqx.exe'
    Assert-Version $nd300 $Version
    Assert-Version $speedqx $Version

    $marker = Get-Marker
    $records = @(Get-ArpRecords)
    if ($ForOrigin -eq 'cargo') {
        if ($marker) { throw "Cargo install unexpectedly has marker $marker" }
        if ($records.Count -ne 0) { throw "Cargo install unexpectedly has $($records.Count) ARP record(s)" }
        $cargoRegistry = Join-Path (Split-Path -Parent (Get-CargoBin)) '.crates2.json'
        if (-not (Test-Path -LiteralPath $cargoRegistry -PathType Leaf)) {
            throw "Cargo install registry is missing: $cargoRegistry"
        }
        $registry = Get-Content -LiteralPath $cargoRegistry -Raw | ConvertFrom-Json
        $keys = @($registry.installs.PSObject.Properties.Name)
        if (-not ($keys | Where-Object { $_ -match "^nd300 $([regex]::Escape($Version)) " })) {
            throw "Cargo registry does not own exact nd300 $Version (keys: $($keys -join ', '))"
        }
    } else {
        $expectedMarker = Get-OriginMarker $ForOrigin
        if ($ExpectNoMarker) {
            if ($marker) { throw "Legacy baseline unexpectedly has marker $marker" }
        } elseif ($marker -ne $expectedMarker) {
            throw "Expected marker $expectedMarker, found $marker"
        }
        if ($records.Count -ne 1) {
            throw "Expected one installer ARP record, found $($records.Count)"
        }
        $machinePath = [Environment]::GetEnvironmentVariable('Path', 'Machine')
        $userPath = [Environment]::GetEnvironmentVariable('Path', 'User')
        if ($ForOrigin.EndsWith('global')) {
            if (-not (Test-PathListContains $machinePath $bin)) {
                throw "Global installer did not register $bin in machine PATH"
            }
        } elseif (-not (Test-PathListContains $userPath $bin)) {
            throw "Corporate installer did not register $bin in user PATH"
        }
    }

    Refresh-ProcessPath
    $resolvedNd300 = (Get-Command nd300.exe -ErrorAction Stop).Source
    $resolvedSpeedqx = (Get-Command speedqx.exe -ErrorAction Stop).Source
    if ((Normalize-PathText $resolvedNd300) -ne (Normalize-PathText $nd300)) {
        throw "nd300 resolves to $resolvedNd300 instead of $nd300"
    }
    if ((Normalize-PathText $resolvedSpeedqx) -ne (Normalize-PathText $speedqx)) {
        throw "speedqx resolves to $resolvedSpeedqx instead of $speedqx"
    }
}

function Get-AssetName {
    param([string]$ForOrigin, [switch]$LegacyName)
    if (-not $ForOrigin) { $ForOrigin = $Origin }
    if ($LegacyName) {
        if ($ForOrigin -ne 'msi-global') {
            throw 'Only the pre-v3.1 Global MSI uses the legacy asset name'
        }
        return 'nd-300-x86_64-pc-windows-msvc.msi'
    }
    switch ($ForOrigin) {
        'msi-global' { return 'nd300-x86_64-pc-windows-msvc.msi' }
        'msi-corporate' { return 'nd300-x86_64-pc-windows-msvc-corporate.msi' }
        'exe-global' { return 'nd300-x86_64-pc-windows-msvc-setup.exe' }
        'exe-corporate' { return 'nd300-x86_64-pc-windows-msvc-corporate-setup.exe' }
        default { throw "Cargo does not use an installer asset" }
    }
}

function Get-RollbackAssetName {
    switch ($Origin) {
        'msi-global' { return 'nd300-x86_64-pc-windows-msvc-rollback-test.msi' }
        'msi-corporate' { return 'nd300-x86_64-pc-windows-msvc-corporate-rollback-test.msi' }
        'exe-global' { return 'nd300-x86_64-pc-windows-msvc-setup-rollback-test.exe' }
        'exe-corporate' { return 'nd300-x86_64-pc-windows-msvc-corporate-setup-rollback-test.exe' }
        default { throw "Cargo does not use an installer rollback asset" }
    }
}

function Get-RetiredImagePaths {
    $dirs = @((Get-InstallBin))
    if ($Origin.EndsWith('global')) {
        $dirs += Join-Path $env:ProgramFiles 'nd-300\bin'
    }
    return @(
        foreach ($bin in ($dirs | Select-Object -Unique)) {
            Join-Path $bin "nd300.update-old-$ExpectedVersion.exe"
            Join-Path $bin "speedqx.update-old-$ExpectedVersion.exe"
        }
    )
}

function Assert-NoRetiredImages {
    param([Parameter(Mandatory)][string]$Label)
    $remaining = @(Get-RetiredImagePaths | Where-Object { Test-Path -LiteralPath $_ -PathType Leaf })
    if ($remaining.Count -ne 0) {
        throw "$Label left retired update image(s): $($remaining -join ', ')"
    }
}

function New-MsiLogPath {
    param([Parameter(Mandatory)][string]$Label)
    $stem = [regex]::Replace($Label.ToLowerInvariant(), '[^a-z0-9]+', '-').Trim('-')
    if (-not $stem) { $stem = 'installer' }
    return Join-Path $EvidenceDir ("$stem-$([guid]::NewGuid().ToString('N')).msi.log")
}

function Write-MsiLogTail {
    param([Parameter(Mandatory)][string]$Path)
    if (Test-Path -LiteralPath $Path -PathType Leaf) {
        Write-Host "`n===== MSI failure tail: $Path ====="
        Get-Content -LiteralPath $Path -Tail 220 | ForEach-Object { Write-Host $_ }
    }
}

function Install-Artifact {
    param(
        [Parameter(Mandatory)][string]$Asset,
        [Parameter(Mandatory)][string]$Label,
        [string]$ArtifactOrigin,
        [switch]$UseDefaultConsolidation
    )
    if (-not $ArtifactOrigin) { $ArtifactOrigin = $Origin }
    if ($ArtifactOrigin.StartsWith('msi-')) {
        $msiLog = New-MsiLogPath $Label
        $arguments = @('/i', $Asset, '/qn', '/norestart', '/l*v!', $msiLog)
        if (-not $UseDefaultConsolidation) {
            $arguments += @('CLEANCARGO=0', 'CLEANOTHEREDITION=0')
        }
        try {
            Invoke-Checked 'msiexec.exe' $arguments $Label
        } catch {
            Write-MsiLogTail $msiLog
            throw
        }
    } else {
        $arguments = @('/VERYSILENT', '/SUPPRESSMSGBOXES', '/NORESTART', '/SP-')
        if (-not $UseDefaultConsolidation) {
            $arguments += '/TASKS="!cleancargo,!cleanotheredition"'
        }
        Invoke-Checked $Asset $arguments $Label
    }
}

function Install-ArtifactExpectFailure {
    param(
        [Parameter(Mandatory)][string]$Asset,
        [Parameter(Mandatory)][string]$Label,
        [string]$ArtifactOrigin
    )
    if (-not $ArtifactOrigin) { $ArtifactOrigin = $Origin }
    if ($ArtifactOrigin.StartsWith('msi-')) {
        $msiLog = New-MsiLogPath $Label
        $result = Invoke-Captured 'msiexec.exe' @(
            '/i', $Asset, '/qn', '/norestart', '/l*v!', $msiLog,
            'CLEANCARGO=0', 'CLEANOTHEREDITION=0'
        ) $Label
    } else {
        $result = Invoke-Captured $Asset @(
            '/VERYSILENT', '/SUPPRESSMSGBOXES', '/NORESTART', '/SP-',
            '/TASKS="!cleancargo,!cleanotheredition"'
        ) $Label
    }
    if ($result.ExitCode -eq 0) {
        throw "$Label unexpectedly succeeded"
    }
    Write-Host "$Label failed as intended with exit $($result.ExitCode)"
}

function Start-SuspendedInstalledImage {
    param([Parameter(Mandatory)][string]$Executable)
    if (-not (Test-Path -LiteralPath $Executable -PathType Leaf)) {
        throw "Cannot start missing running-image fixture: $Executable"
    }
    $image = [Nd300SuspendedImage]::Start($Executable)
    if (-not $image.IsAlive) {
        $image.Dispose()
        throw 'Suspended running-image fixture exited before the update'
    }
    Write-Host "Mapped baseline updater image in suspended PID $($image.ProcessId)"
    return $image
}

function Install-CargoRelease {
    param([Parameter(Mandatory)][string]$Version, [Parameter(Mandatory)][string]$Label)
    Invoke-Checked 'cargo.exe' @('install', 'nd300', '--version', "=$Version", '--locked', '--force') $Label
}

function Install-CargoCandidate {
    Invoke-Checked 'cargo.exe' @('install', '--path', $RepoRoot, '--locked', '--force') 'Install local Cargo candidate'
}

function Download-PublishedAsset {
    param(
        [Parameter(Mandatory)][string]$Version,
        [Parameter(Mandatory)][string]$ForOrigin,
        [switch]$LegacyName
    )
    $assetDir = Join-Path $env:RUNNER_TEMP "nd300-published-$Version-$ForOrigin"
    New-Item -ItemType Directory -Force -Path $assetDir | Out-Null
    $name = Get-AssetName -ForOrigin $ForOrigin -LegacyName:$LegacyName
    Invoke-Checked 'gh.exe' @('release', 'download', "v$Version", '--repo', $Repository, '--pattern', $name, '--dir', $assetDir) "Download published $name"
    $path = Join-Path $assetDir $name
    if (-not (Test-Path -LiteralPath $path -PathType Leaf)) {
        throw "Downloaded published asset is missing: $path"
    }
    return $path
}

function Wait-Until {
    param(
        [Parameter(Mandatory)][scriptblock]$Condition,
        [Parameter(Mandatory)][string]$Description,
        [int]$TimeoutSeconds = 180
    )
    $deadline = (Get-Date).AddSeconds($TimeoutSeconds)
    do {
        if (& $Condition) { return }
        Start-Sleep -Seconds 2
    } while ((Get-Date) -lt $deadline)
    throw "Timed out waiting for $Description"
}

function Assert-CargoOverridesStaleMarker {
    param([Parameter(Mandatory)][string]$CargoExe)
    New-Item -ItemType Directory -Force -Path 'HKCU:\Software\ND300' | Out-Null
    Set-ItemProperty -LiteralPath 'HKCU:\Software\ND300' -Name InstallSource -Value 'msi-global' -Type String
    $result = Invoke-Captured $CargoExe @('update', '--json') 'Probe Cargo ownership with a stale marker'
    if ($result.ExitCode -ne 0) { throw "Cargo stale-marker update probe failed with exit $($result.ExitCode)" }
    $payload = $result.StdOut | ConvertFrom-Json
    if ($payload.install_origin -ne 'cargo-or-installer') {
        throw "Stale marker won over Cargo path: install_origin=$($payload.install_origin)"
    }
    Remove-ItemProperty -LiteralPath 'HKCU:\Software\ND300' -Name InstallSource -ErrorAction SilentlyContinue
}

New-Item -ItemType Directory -Force -Path $EvidenceDir | Out-Null
$RepoRoot = (Resolve-Path -LiteralPath $RepoRoot).Path
if (-not $BaselineOrigin) { $BaselineOrigin = $Origin }
if ($Scenario -eq 'lifecycle' -and $BaselineOrigin -ne $Origin) {
    throw 'Lifecycle scenarios must preserve one origin; use Scenario=takeover for a fresh channel change'
}
if ($LegacyGlobalBaseline -and $BaselineOrigin -ne 'msi-global') {
    throw 'LegacyGlobalBaseline is valid only for a Global MSI baseline'
}
if ($Scenario -in @('takeover', 'refusal') -and $Origin -eq 'cargo') {
    throw 'A bare cargo install has no post-install hook; takeover targets must be official installers'
}

$installBin = Get-InstallBin $Origin
$baselineInstallBin = if ($LegacyGlobalBaseline) {
    Join-Path $env:ProgramFiles 'nd-300\bin'
} else {
    Get-InstallBin $BaselineOrigin
}
$installedNd300 = Join-Path $installBin 'nd300.exe'
$baselineNd300 = Join-Path $baselineInstallBin 'nd300.exe'

Write-Host "Origin=$Origin BaselineOrigin=$BaselineOrigin Scenario=$Scenario Mode=$Mode Baseline=$BaselineVersion Expected=$ExpectedVersion"

# 1. Published baseline.
if ($BaselineOrigin -eq 'cargo') {
    Install-CargoRelease $BaselineVersion 'Install Cargo baseline'
} else {
    $baselineAsset = Download-PublishedAsset `
        -Version $BaselineVersion `
        -ForOrigin $BaselineOrigin `
        -LegacyName:$LegacyGlobalBaseline
    Install-Artifact $baselineAsset 'Install published baseline artifact' $BaselineOrigin
    Wait-Until {
        (Test-Path -LiteralPath $baselineNd300 -PathType Leaf) -and
        (Test-Path -LiteralPath (Join-Path $baselineInstallBin 'speedqx.exe') -PathType Leaf)
    } 'baseline installer to materialize both binaries' 60
}
Assert-InstallState `
    -Version $BaselineVersion `
    -ForOrigin $BaselineOrigin `
    -InstallBin $baselineInstallBin `
    -ExpectNoMarker:$LegacyGlobalBaseline
Write-Snapshot 'baseline' $baselineInstallBin

# Raw Windows installers cannot safely switch between per-user and per-machine
# scopes inside an active MSI/elevated transaction. They must refuse before any
# binary, marker, ARP, or PATH mutation instead of executing a user-writable
# uninstaller across an elevation boundary.
if ($Scenario -eq 'refusal') {
    if ($Origin -eq $BaselineOrigin) {
        throw 'Refusal scenarios require different baseline and target origins'
    }
    if ($Mode -eq 'candidate') {
        if (-not $CandidateDir) { throw 'CandidateDir is required in candidate mode' }
        $targetAsset = Join-Path $CandidateDir (Get-AssetName $Origin)
        if (-not (Test-Path -LiteralPath $targetAsset -PathType Leaf)) {
            throw "Candidate asset is missing: $targetAsset"
        }
    } else {
        $targetAsset = Download-PublishedAsset -Version $ExpectedVersion -ForOrigin $Origin
    }

    $baselineFingerprint = Get-InstallFingerprint $baselineInstallBin
    Install-ArtifactExpectFailure $targetAsset "Refuse unsafe $BaselineOrigin to $Origin scope switch"
    Assert-FingerprintEqual $baselineFingerprint (Get-InstallFingerprint $baselineInstallBin) 'Cross-scope refusal'
    Assert-InstallState $BaselineVersion $BaselineOrigin $baselineInstallBin
    Assert-NoRetiredImages 'Cross-scope refusal'
    Write-Snapshot "refused-$Origin" $baselineInstallBin

    # The refusal deliberately leaves the published old binary in control. Do
    # not ask that old version to prove the new installer-aware uninstall path;
    # tear down the disposable fixture through its exact MSI registration.
    if (-not $BaselineOrigin.StartsWith('msi-')) {
        throw 'The refusal cleanup currently requires an MSI baseline'
    }
    $baselineRecords = @(
        Get-ArpRecords | Where-Object {
            (Get-PropertyValue -InputObject $_ -Name 'WindowsInstaller') -eq 1
        }
    )
    if ($baselineRecords.Count -ne 1) {
        throw "Expected exactly one refusal-baseline MSI registration, found $($baselineRecords.Count)"
    }
    $baselineProductCode = Get-PropertyValue -InputObject $baselineRecords[0] -Name 'PSChildName'
    if ($baselineProductCode -notmatch '^\{[0-9A-Fa-f-]{36}\}$') {
        throw "Refusal-baseline MSI registration has an invalid product code: $baselineProductCode"
    }
    $refusalUninstallLog = New-MsiLogPath 'Uninstall refusal baseline'
    Invoke-Checked 'msiexec.exe' @(
        '/x', $baselineProductCode, '/qn', '/norestart', '/l*v!', $refusalUninstallLog
    ) 'Uninstall refusal baseline through its MSI registration'
    Wait-Until {
        -not (Test-Path -LiteralPath $baselineNd300 -PathType Leaf) -and
        @(Get-ArpRecords).Count -eq 0 -and
        -not (Get-Marker)
    } 'refusal baseline uninstaller to remove all registered ownership' 180
    Write-Host "PASS: $BaselineOrigin -> $Origin $Mode Windows cross-scope safe refusal"
    exit 0
}

# A fresh install through another channel represents the user's latest intent.
# Prove representative channel changes consolidate ARP, PATH, marker, and binary
# ownership instead of leaving two active installs.
if ($Scenario -eq 'takeover') {
    if ($Origin -eq $BaselineOrigin) {
        throw 'Takeover scenarios require different baseline and target origins'
    }
    if ($Mode -eq 'candidate') {
        if (-not $CandidateDir) { throw 'CandidateDir is required in candidate mode' }
        $targetAsset = Join-Path $CandidateDir (Get-AssetName $Origin)
        if (-not (Test-Path -LiteralPath $targetAsset -PathType Leaf)) {
            throw "Candidate asset is missing: $targetAsset"
        }
    } else {
        $targetAsset = Download-PublishedAsset -Version $ExpectedVersion -ForOrigin $Origin
    }

    Install-Artifact `
        -Asset $targetAsset `
        -Label "Fresh install takes over from $BaselineOrigin" `
        -ArtifactOrigin $Origin `
        -UseDefaultConsolidation
    Wait-Until {
        (Test-Path -LiteralPath $installedNd300 -PathType Leaf) -and
        (Test-Path -LiteralPath (Join-Path $installBin 'speedqx.exe') -PathType Leaf)
    } 'target installer to materialize both binaries' 90
    Assert-InstallState $ExpectedVersion $Origin $installBin

    if ((Normalize-PathText $baselineInstallBin) -ne (Normalize-PathText $installBin)) {
        if ((Test-Path -LiteralPath $baselineNd300 -PathType Leaf) -or
            (Test-Path -LiteralPath (Join-Path $baselineInstallBin 'speedqx.exe') -PathType Leaf)) {
            throw "Fresh $Origin install left binaries in prior channel directory $baselineInstallBin"
        }
        if ($BaselineOrigin -ne 'cargo') {
            $machinePath = [Environment]::GetEnvironmentVariable('Path', 'Machine')
            $userPath = [Environment]::GetEnvironmentVariable('Path', 'User')
            if ((Test-PathListContains $machinePath $baselineInstallBin) -or
                (Test-PathListContains $userPath $baselineInstallBin)) {
                throw "Fresh $Origin install left prior channel PATH ownership for $baselineInstallBin"
            }
        }
    }
    Write-Snapshot "takeover-from-$BaselineOrigin"

    $uninstallResult = Invoke-Captured $installedNd300 @('uninstall', '--json') 'Uninstall takeover target'
    if ($uninstallResult.ExitCode -ne 0) {
        throw "Takeover target uninstall failed with exit $($uninstallResult.ExitCode)"
    }
    Wait-Until {
        -not (Test-Path -LiteralPath $installedNd300 -PathType Leaf) -and
        @(Get-ArpRecords).Count -eq 0 -and
        -not (Get-Marker)
    } 'takeover target uninstaller to remove all registered ownership' 180
    Write-Snapshot 'takeover-uninstalled'
    Write-Host "PASS: $BaselineOrigin -> $Origin $Mode Windows fresh-channel takeover"
    exit 0
}

# 2. Candidate artifact or public self-update. Installer candidates first prove
# deterministic rollback after running-image retirement starts, then prove a
# successful replacement without terminating that same process.
$liveImage = $null
try {
    if ($Mode -eq 'candidate') {
        if ($Origin -eq 'cargo') {
            Install-CargoCandidate
        } else {
            if (-not $CandidateDir) { throw 'CandidateDir is required in candidate mode' }
            $candidateAsset = Join-Path $CandidateDir (Get-AssetName)
            $rollbackAsset = Join-Path $CandidateDir (Get-RollbackAssetName)
            foreach ($asset in @($candidateAsset, $rollbackAsset)) {
                if (-not (Test-Path -LiteralPath $asset -PathType Leaf)) {
                    throw "Candidate asset is missing: $asset"
                }
            }

            $baselineFingerprint = Get-InstallFingerprint $baselineInstallBin
            $liveImage = Start-SuspendedInstalledImage $baselineNd300
            Install-ArtifactExpectFailure $rollbackAsset 'Inject candidate failure after running-image retirement'
            if (-not $liveImage.IsAlive) {
                throw 'Faulting candidate installer terminated the running baseline updater'
            }
            Assert-InstallState `
                -Version $BaselineVersion `
                -ForOrigin $BaselineOrigin `
                -InstallBin $baselineInstallBin `
                -ExpectNoMarker:$LegacyGlobalBaseline
            Assert-FingerprintEqual $baselineFingerprint (Get-InstallFingerprint $baselineInstallBin) 'Faulting candidate installer'
            Assert-NoRetiredImages 'Faulting candidate installer rollback'
            Write-Snapshot 'rollback-restored' $baselineInstallBin

            Install-Artifact $candidateAsset 'Upgrade to locally built candidate artifact'
            if (-not $liveImage.IsAlive) {
                throw 'Successful candidate installer terminated the running baseline updater'
            }
            Assert-InstallState $ExpectedVersion
            $retiredNd300 = @(Get-RetiredImagePaths | Where-Object {
                $_ -like '*\nd300.update-old-*.exe' -and
                (Test-Path -LiteralPath $_ -PathType Leaf)
            })
            if ($retiredNd300.Count -ne 1) {
                throw "Successful update expected one mapped retired nd300 image, found $($retiredNd300.Count)"
            }
            [ordered]@{
                origin = $Origin
                running_image_pid = $liveImage.ProcessId
                running_image_survived = $liveImage.IsAlive
                retired_image = $retiredNd300[0]
                retired_image_exists_while_running = $true
            } | ConvertTo-Json | Set-Content `
                -LiteralPath (Join-Path $EvidenceDir "$Origin-running-image.json") `
                -Encoding utf8NoBOM
            Write-Snapshot 'upgraded-live-image'

            $liveImage.Stop()
            $liveImage = $null
            Wait-Until {
                @(Get-RetiredImagePaths | Where-Object {
                    Test-Path -LiteralPath $_ -PathType Leaf
                }).Count -eq 0
            } 'retired running image cleanup after updater exit' 60
            Assert-NoRetiredImages 'Successful candidate installer cleanup'
        }
    } else {
        # v2.9 predates clap subcommands but already supports the legacy
        # --update action flag. Current baselines exercise the preferred form;
        # the legacy case must invoke the interface that binary actually ships.
        $updateArgs = if ($LegacyGlobalBaseline) {
            @('--update', '--json')
        } else {
            @('update', '--json')
        }
        $result = Invoke-Captured $baselineNd300 $updateArgs 'Run public nd300 update'
        $result | ConvertTo-Json | Set-Content `
            -LiteralPath (Join-Path $EvidenceDir "$Origin-public-update-process.json") `
            -Encoding utf8NoBOM
        if ($result.ExitCode -ne 0) {
            Write-Host "::error::Public nd300 update failed with exit $($result.ExitCode)"
            throw "Public nd300 update failed with exit $($result.ExitCode)"
        }
    }
} finally {
    if ($liveImage) {
        $liveImage.Stop()
    }
}

Assert-InstallState $ExpectedVersion
Write-Snapshot 'upgraded'

# 3. Hidden migration interface must be dry-run safe and accept all origins.
$migrationArgs = @('migrate-cleanup', '--json', '--dry-run', '--cargo-copy', '--other-edition')
if ($Origin -ne 'cargo') { $migrationArgs += @('--install-origin', $Origin) }
$migrationResult = Invoke-Captured $installedNd300 $migrationArgs 'Run dry-run migration'
if ($migrationResult.ExitCode -ne 0) { throw "Dry-run migration failed with exit $($migrationResult.ExitCode)" }
$migration = $migrationResult.StdOut | ConvertFrom-Json
if (-not $migration.dry_run) { throw 'Migration report did not preserve dry_run=true' }
Assert-InstallState $ExpectedVersion

# 4. Registered installs uninstall through their proven owner while a Cargo copy
# survives. Then a deliberately stale marker must still classify that Cargo path.
$cargoBin = Get-CargoBin
$cargoNd300 = Join-Path $cargoBin 'nd300.exe'
$cargoSpeedqx = Join-Path $cargoBin 'speedqx.exe'
if ($Origin -ne 'cargo') {
    if ($Mode -eq 'candidate') { Install-CargoCandidate } else { Install-CargoRelease $ExpectedVersion 'Install surviving Cargo copy' }
    Assert-Version $cargoNd300 $ExpectedVersion

    $uninstallResult = Invoke-Captured $installedNd300 @('uninstall', '--json') 'Uninstall registered origin'
    if ($uninstallResult.ExitCode -ne 0) { throw "Registered uninstall launch failed with exit $($uninstallResult.ExitCode)" }

    Wait-Until {
        $machinePath = [Environment]::GetEnvironmentVariable('Path', 'Machine')
        $userPath = [Environment]::GetEnvironmentVariable('Path', 'User')
        -not (Test-Path -LiteralPath $installedNd300 -PathType Leaf) -and
        @(Get-ArpRecords).Count -eq 0 -and
        -not (Get-Marker) -and
        -not (Test-PathListContains $machinePath $installBin) -and
        -not (Test-PathListContains $userPath $installBin)
    } 'registered uninstaller to remove binaries, ARP, marker, and PATH ownership'

    $machinePath = [Environment]::GetEnvironmentVariable('Path', 'Machine')
    $userPath = [Environment]::GetEnvironmentVariable('Path', 'User')
    if ((Test-PathListContains $machinePath $installBin) -or (Test-PathListContains $userPath $installBin)) {
        throw "Registered uninstaller left install bin on PATH: $installBin"
    }
    Assert-Version $cargoNd300 $ExpectedVersion
}

Assert-CargoOverridesStaleMarker $cargoNd300
Write-Snapshot 'cargo-survivor'

# 5. Cargo/portable uninstall remains allowlisted and leaves the toolchain.
$cargoUninstallResult = Invoke-Captured $cargoNd300 @('uninstall', '--json') 'Uninstall Cargo origin'
if ($cargoUninstallResult.ExitCode -ne 0) { throw "Cargo uninstall failed with exit $($cargoUninstallResult.ExitCode)" }
Wait-Until {
    -not (Test-Path -LiteralPath $cargoNd300 -PathType Leaf) -and
    -not (Test-Path -LiteralPath $cargoSpeedqx -PathType Leaf)
} 'Cargo binary-pair removal' 60
if (-not (Test-Path -LiteralPath (Join-Path $cargoBin 'cargo.exe') -PathType Leaf)) {
    throw 'Cargo uninstall removed cargo.exe'
}
if (-not (Test-Path -LiteralPath (Join-Path $cargoBin 'rustup.exe') -PathType Leaf)) {
    throw 'Cargo uninstall removed rustup.exe'
}
if (Get-Marker) { throw 'Final uninstall left an installer marker' }
Write-Snapshot 'uninstalled'

Write-Host "PASS: $Origin $Mode Windows origin matrix"
