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
    switch ($Origin) {
        'msi-global' { return Join-Path $env:ProgramFiles 'nd300\bin' }
        'exe-global' { return Join-Path $env:ProgramFiles 'nd300\bin' }
        'msi-corporate' { return Join-Path $env:LOCALAPPDATA 'Programs\nd300\bin' }
        'exe-corporate' { return Join-Path $env:LOCALAPPDATA 'Programs\nd300\bin' }
        'cargo' { return Get-CargoBin }
    }
}

function Get-OriginMarker {
    if ($Origin -eq 'cargo') { return $null }
    return $Origin
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
                    @('nd300', 'nd300 (Corporate Edition)')
            }
    )
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
    $lines = @(& $Executable --version 2>&1)
    $code = $LASTEXITCODE
    $text = $lines -join "`n"
    Write-Host $text
    if ($code -ne 0 -or $text -notmatch "(?<![0-9])$([regex]::Escape($Version))(?![0-9])") {
        throw "$Executable did not report expected version $Version (exit=$code, output=$text)"
    }
}

function Write-Snapshot {
    param([Parameter(Mandatory)][string]$Label)
    New-Item -ItemType Directory -Force -Path $EvidenceDir | Out-Null
    Refresh-ProcessPath
    $bin = Get-InstallBin
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
        mode = $Mode
        baseline_version = $BaselineVersion
        expected_version = $ExpectedVersion
        marker = Get-Marker
        arp = $arp
        install_bin = $bin
        nd300_exists = Test-Path -LiteralPath $nd300 -PathType Leaf
        speedqx_exists = Test-Path -LiteralPath $speedqx -PathType Leaf
        nd300_sha256 = if (Test-Path -LiteralPath $nd300 -PathType Leaf) { (Get-FileHash -LiteralPath $nd300 -Algorithm SHA256).Hash.ToLowerInvariant() } else { $null }
        resolved_nd300 = if ($resolved) { $resolved.Source } else { $null }
        machine_path_has_install_bin = Test-PathListContains $machinePath $bin
        user_path_has_install_bin = Test-PathListContains $userPath $bin
    }
    $path = Join-Path $EvidenceDir "$Origin-$Label.json"
    $snapshot | ConvertTo-Json -Depth 8 | Set-Content -LiteralPath $path -Encoding utf8NoBOM
    Write-Host "Evidence: $path"
}

function Assert-InstallState {
    param([Parameter(Mandatory)][string]$Version)
    $bin = Get-InstallBin
    $nd300 = Join-Path $bin 'nd300.exe'
    $speedqx = Join-Path $bin 'speedqx.exe'
    Assert-Version $nd300 $Version
    Assert-Version $speedqx $Version

    $marker = Get-Marker
    $records = @(Get-ArpRecords)
    if ($Origin -eq 'cargo') {
        if ($marker) { throw "Cargo install unexpectedly has marker $marker" }
        if ($records.Count -ne 0) { throw "Cargo install unexpectedly has $($records.Count) ARP record(s)" }
    } else {
        $expectedMarker = Get-OriginMarker
        if ($marker -ne $expectedMarker) {
            throw "Expected marker $expectedMarker, found $marker"
        }
        if ($records.Count -ne 1) {
            throw "Expected one installer ARP record, found $($records.Count)"
        }
        $machinePath = [Environment]::GetEnvironmentVariable('Path', 'Machine')
        $userPath = [Environment]::GetEnvironmentVariable('Path', 'User')
        if ($Origin.EndsWith('global')) {
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
    switch ($Origin) {
        'msi-global' { return 'nd300-x86_64-pc-windows-msvc.msi' }
        'msi-corporate' { return 'nd300-x86_64-pc-windows-msvc-corporate.msi' }
        'exe-global' { return 'nd300-x86_64-pc-windows-msvc-setup.exe' }
        'exe-corporate' { return 'nd300-x86_64-pc-windows-msvc-corporate-setup.exe' }
        default { throw "Cargo does not use an installer asset" }
    }
}

function Install-Artifact {
    param([Parameter(Mandatory)][string]$Asset, [Parameter(Mandatory)][string]$Label)
    if ($Origin.StartsWith('msi-')) {
        Invoke-Checked 'msiexec.exe' @('/i', $Asset, '/qn', '/norestart', 'CLEANCARGO=0', 'CLEANOTHEREDITION=0') $Label
    } else {
        Invoke-Checked $Asset @('/VERYSILENT', '/SUPPRESSMSGBOXES', '/NORESTART', '/SP-', '/TASKS="!cleancargo,!cleanotheredition"') $Label
    }
}

function Install-CargoRelease {
    param([Parameter(Mandatory)][string]$Version, [Parameter(Mandatory)][string]$Label)
    Invoke-Checked 'cargo.exe' @('install', 'nd300', '--version', "=$Version", '--locked', '--force') $Label
}

function Install-CargoCandidate {
    Invoke-Checked 'cargo.exe' @('install', '--path', $RepoRoot, '--locked', '--force') 'Install local Cargo candidate'
}

function Download-BaselineAsset {
    $assetDir = Join-Path $env:RUNNER_TEMP "nd300-baseline-$Origin"
    New-Item -ItemType Directory -Force -Path $assetDir | Out-Null
    $name = Get-AssetName
    Invoke-Checked 'gh.exe' @('release', 'download', "v$BaselineVersion", '--repo', $Repository, '--pattern', $name, '--dir', $assetDir) "Download baseline $name"
    $path = Join-Path $assetDir $name
    if (-not (Test-Path -LiteralPath $path -PathType Leaf)) {
        throw "Downloaded baseline asset is missing: $path"
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
    $lines = @(& $CargoExe update --json 2>&1)
    $code = $LASTEXITCODE
    $text = $lines -join "`n"
    Write-Host $text
    if ($code -ne 0) { throw "Cargo stale-marker update probe failed with exit $code" }
    $payload = $text | ConvertFrom-Json
    if ($payload.install_origin -ne 'cargo-or-installer') {
        throw "Stale marker won over Cargo path: install_origin=$($payload.install_origin)"
    }
    Remove-ItemProperty -LiteralPath 'HKCU:\Software\ND300' -Name InstallSource -ErrorAction SilentlyContinue
}

New-Item -ItemType Directory -Force -Path $EvidenceDir | Out-Null
$RepoRoot = (Resolve-Path -LiteralPath $RepoRoot).Path
$installBin = Get-InstallBin
$installedNd300 = Join-Path $installBin 'nd300.exe'

Write-Host "Origin=$Origin Mode=$Mode Baseline=$BaselineVersion Expected=$ExpectedVersion"

# 1. Published baseline.
if ($Origin -eq 'cargo') {
    Install-CargoRelease $BaselineVersion 'Install Cargo baseline'
} else {
    $baselineAsset = Download-BaselineAsset
    Install-Artifact $baselineAsset 'Install published baseline artifact'
    Wait-Until {
        (Test-Path -LiteralPath $installedNd300 -PathType Leaf) -and
        (Test-Path -LiteralPath (Join-Path $installBin 'speedqx.exe') -PathType Leaf)
    } 'baseline installer to materialize both binaries' 60
}
Assert-InstallState $BaselineVersion
Write-Snapshot 'baseline'

# 2. Candidate artifact or public self-update.
if ($Mode -eq 'candidate') {
    if ($Origin -eq 'cargo') {
        Install-CargoCandidate
    } else {
        if (-not $CandidateDir) { throw 'CandidateDir is required in candidate mode' }
        $candidateAsset = Join-Path $CandidateDir (Get-AssetName)
        if (-not (Test-Path -LiteralPath $candidateAsset -PathType Leaf)) {
            throw "Candidate asset is missing: $candidateAsset"
        }
        Install-Artifact $candidateAsset 'Upgrade to locally built candidate artifact'
    }
} else {
    $lines = @(& $installedNd300 update --json 2>&1)
    $code = $LASTEXITCODE
    $lines | ForEach-Object { Write-Host $_ }
    if ($code -ne 0) { throw "Public nd300 update failed with exit $code" }
}

Assert-InstallState $ExpectedVersion
Write-Snapshot 'upgraded'

# 3. Hidden migration interface must be dry-run safe and accept all origins.
$migrationArgs = @('migrate-cleanup', '--json', '--dry-run', '--cargo-copy', '--other-edition')
if ($Origin -ne 'cargo') { $migrationArgs += @('--install-origin', $Origin) }
$migrationLines = @(& $installedNd300 @migrationArgs 2>&1)
$migrationCode = $LASTEXITCODE
$migrationText = $migrationLines -join "`n"
Write-Host $migrationText
if ($migrationCode -ne 0) { throw "Dry-run migration failed with exit $migrationCode" }
$migration = $migrationText | ConvertFrom-Json
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

    $uninstallLines = @(& $installedNd300 uninstall --json 2>&1)
    $uninstallCode = $LASTEXITCODE
    $uninstallLines | ForEach-Object { Write-Host $_ }
    if ($uninstallCode -ne 0) { throw "Registered uninstall launch failed with exit $uninstallCode" }

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
$cargoUninstallLines = @(& $cargoNd300 uninstall --json 2>&1)
$cargoUninstallCode = $LASTEXITCODE
$cargoUninstallLines | ForEach-Object { Write-Host $_ }
if ($cargoUninstallCode -ne 0) { throw "Cargo uninstall failed with exit $cargoUninstallCode" }
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
