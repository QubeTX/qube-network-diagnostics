[CmdletBinding()]
param()

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

function Assert-True {
    param([Parameter(Mandatory)][bool]$Condition, [Parameter(Mandatory)][string]$Message)
    if (-not $Condition) { throw $Message }
}

function New-PatchedFixture {
    param([Parameter(Mandatory)][string]$Path)

    @'
$app_version = '9.8.7'
function Invoke-Installer($artifacts, $dest_dir, $dest_dir_lib) {
  $dest_dir = New-Item -Force -ItemType Directory -Path $dest_dir
  $dest_dir_lib = New-Item -Force -ItemType Directory -Path $dest_dir_lib
  Write-Information "installing to $dest_dir"
  # Just copy the binaries from the temp location to the install dir
  foreach ($bin_path in $artifacts["bin_paths"]) {
    $installed_file = Split-Path -Path "$bin_path" -Leaf
    Copy-Item "$bin_path" -Destination "$dest_dir" -ErrorAction Stop
  }
  foreach ($lib_path in $artifacts["lib_paths"]) {
    Copy-Item "$lib_path" -Destination "$dest_dir_lib" -ErrorAction Stop
  }
}
'@ | Set-Content -LiteralPath $Path -Encoding utf8NoBOM

    & "$PSScriptRoot\harden-cargo-dist-windows-installer.ps1" `
        -Path $Path `
        -ExpectedVersion '9.8.7'
}

function Start-LockedImage {
    param([Parameter(Mandatory)][string]$Path)

    Copy-Item -LiteralPath "$env:SystemRoot\System32\ping.exe" -Destination $Path -Force
    $startInfo = [System.Diagnostics.ProcessStartInfo]::new()
    $startInfo.FileName = $Path
    $startInfo.UseShellExecute = $false
    $startInfo.CreateNoWindow = $true
    $startInfo.RedirectStandardOutput = $true
    $startInfo.RedirectStandardError = $true
    foreach ($argument in @('-n', '60', '127.0.0.1')) {
        [void]$startInfo.ArgumentList.Add($argument)
    }
    $process = [System.Diagnostics.Process]::Start($startInfo)
    if (-not $process) { throw "Could not start locked-image fixture $Path" }
    Start-Sleep -Milliseconds 150
    Assert-True (-not $process.HasExited) "Locked-image fixture exited before the test: $Path"
    return $process
}

function Stop-TestProcess {
    param([AllowNull()][System.Diagnostics.Process]$Process)
    if ($Process -and -not $Process.HasExited) {
        $Process.Kill()
        $Process.WaitForExit()
    }
    if ($Process) { $Process.Dispose() }
}

$temp = Join-Path ([System.IO.Path]::GetTempPath()) ("nd300-installer-patch-" + [guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path $temp | Out-Null
$liveProcess = $null
$staleProcess = $null
try {
    $fixture = Join-Path $temp 'nd300-installer.ps1'
    New-PatchedFixture $fixture
    $patched = Get-Content -LiteralPath $fixture -Raw
    foreach ($required in @(
        'ND300 running-image compatibility shim',
        'Select-Nd300UpdateChannel',
        'Select-Nd300InvocationMode',
        'Test-Nd300InstallLocationOwnsParent',
        '[Environment]::SystemDirectory',
        'releases/download/v$app_version',
        'install-takeover --target standalone --scope all',
        'nd300.exe", "speedqx.exe',
        'migrate-cleanup --quiet --retired-update',
        'rollback also failed',
        'throw $install_error'
    )) {
        Assert-True $patched.Contains($required) "Patched fixture is missing: $required"
    }

    # The post-processor is intentionally single-use and version-bound.
    $failedClosed = $false
    try {
        & "$PSScriptRoot\harden-cargo-dist-windows-installer.ps1" `
            -Path $fixture `
            -ExpectedVersion '9.8.7'
    } catch {
        $failedClosed = $_.Exception.Message -match 'already contains'
    }
    Assert-True $failedClosed 'A second patch pass did not fail closed'

    $unsafeVersionRejected = $false
    $freshFixture = Join-Path $temp 'unsafe-version.ps1'
    Copy-Item -LiteralPath $fixture -Destination $freshFixture
    try {
        & "$PSScriptRoot\harden-cargo-dist-windows-installer.ps1" `
            -Path $freshFixture `
            -ExpectedVersion '..\escape'
    } catch {
        $unsafeVersionRejected = $_.Exception.Message -match 'unsafe installer version'
    }
    Assert-True $unsafeVersionRejected 'Unsafe retired-image version was accepted'

    # Success path: keep an old executable genuinely running, replace both
    # canonical files, and prove the old process survives the rename.
    . $fixture
    $script:ND300InstallerFixtureMode = $true
    Assert-True ((Select-Nd300UpdateChannel 'nd300' @('msi-global') $null) -eq 'msi-global') 'A legacy updater did not preserve its unique registered channel'
    Assert-True ($null -eq (Select-Nd300UpdateChannel 'powershell' @('msi-global') $null)) 'A manual fresh PowerShell install was incorrectly treated as an update'
    Assert-True ((Select-Nd300UpdateChannel 'speedqx' @('msi-global', 'exe-global') 'exe-global') -eq 'exe-global') 'The newest marker did not disambiguate registered legacy channels'
    $ownedRoot = Join-Path $temp 'owned-channel'
    $ownedBin = Join-Path $ownedRoot 'bin'
    New-Item -ItemType Directory -Force -Path $ownedBin | Out-Null
    [System.IO.File]::WriteAllText((Join-Path $ownedBin 'nd300.exe'), 'fixture')
    Assert-True (Test-Nd300InstallLocationOwnsParent $ownedRoot (Join-Path $ownedBin 'nd300.exe')) 'A registered InstallLocation did not prove ownership of its bin child'
    Assert-True (-not (Test-Nd300InstallLocationOwnsParent (Join-Path $temp 'unrelated') (Join-Path $ownedBin 'nd300.exe'))) 'An unrelated InstallLocation was accepted as the parent owner'
    Assert-True ((Select-Nd300InvocationMode 'powershell' "$env:SystemRoot\System32\WindowsPowerShell\v1.0\powershell.exe" $ownedBin @() $null $false) -eq 'fresh') 'A manual PowerShell launch was not classified as fresh'
    Assert-True ((Select-Nd300InvocationMode 'nd300' (Join-Path $ownedBin 'nd300.exe') $ownedBin @() $null $false) -eq 'standalone-update') 'An owned standalone parent was not preserved'
    Assert-True ((Select-Nd300InvocationMode 'nd300' (Join-Path $ownedBin 'nd300.exe') $ownedBin @() $null $true) -eq 'cargo-update') 'A Cargo-registered parent was not preserved as Cargo'
    $unownedParentRejected = $false
    try {
        Select-Nd300InvocationMode 'nd300' (Join-Path $temp 'unowned\nd300.exe') $ownedBin @() $null $false | Out-Null
    } catch {
        $unownedParentRejected = $_.Exception.Message -match 'no files were changed'
    }
    Assert-True $unownedParentRejected 'An unowned ND-300 parent did not fail closed'
    $successRoot = Join-Path $temp 'success'
    $successDest = Join-Path $successRoot 'dest'
    $successSource = Join-Path $successRoot 'source'
    $successLib = Join-Path $successRoot 'lib'
    New-Item -ItemType Directory -Force -Path $successDest, $successSource, $successLib | Out-Null
    $liveProcess = Start-LockedImage (Join-Path $successDest 'nd300.exe')
    [System.IO.File]::WriteAllText((Join-Path $successDest 'speedqx.exe'), 'old-speedqx')
    Copy-Item -LiteralPath "$env:SystemRoot\System32\where.exe" -Destination (Join-Path $successSource 'nd300.exe')
    Copy-Item -LiteralPath "$env:SystemRoot\System32\whoami.exe" -Destination (Join-Path $successSource 'speedqx.exe')
    $artifacts = @{
        bin_paths = @(
            (Join-Path $successSource 'nd300.exe'),
            (Join-Path $successSource 'speedqx.exe')
        )
        lib_paths = @()
    }
    Invoke-Installer $artifacts $successDest $successLib
    Assert-True (-not $liveProcess.HasExited) 'Installer terminated the running old image'
    Assert-True (Test-Path -LiteralPath (Join-Path $successDest 'nd300.update-old-9.8.7.exe')) 'Old running image was not retired'
    Assert-True ((Get-FileHash (Join-Path $successDest 'nd300.exe')).Hash -eq (Get-FileHash "$env:SystemRoot\System32\where.exe").Hash) 'New nd300.exe was not installed'
    Assert-True ((Get-FileHash (Join-Path $successDest 'speedqx.exe')).Hash -eq (Get-FileHash "$env:SystemRoot\System32\whoami.exe").Hash) 'New speedqx.exe was not installed'
    Stop-TestProcess $liveProcess
    $liveProcess = $null

    # Copy failure after the first new binary must restore the original pair.
    $rollbackRoot = Join-Path $temp 'rollback'
    $rollbackDest = Join-Path $rollbackRoot 'dest'
    $rollbackSource = Join-Path $rollbackRoot 'source'
    $rollbackLib = Join-Path $rollbackRoot 'lib'
    New-Item -ItemType Directory -Force -Path $rollbackDest, $rollbackSource, $rollbackLib | Out-Null
    [System.IO.File]::WriteAllText((Join-Path $rollbackDest 'nd300.exe'), 'old-nd300')
    [System.IO.File]::WriteAllText((Join-Path $rollbackDest 'speedqx.exe'), 'old-speedqx')
    Copy-Item -LiteralPath "$env:SystemRoot\System32\where.exe" -Destination (Join-Path $rollbackSource 'nd300.exe')
    $rollbackFailed = $false
    try {
        Invoke-Installer @{
            bin_paths = @(
                (Join-Path $rollbackSource 'nd300.exe'),
                (Join-Path $rollbackSource 'missing-speedqx.exe')
            )
            lib_paths = @()
        } $rollbackDest $rollbackLib
    } catch {
        $rollbackFailed = $true
    }
    Assert-True $rollbackFailed 'Injected copy failure unexpectedly succeeded'
    Assert-True ((Get-Content (Join-Path $rollbackDest 'nd300.exe') -Raw) -eq 'old-nd300') 'nd300.exe was not restored after copy failure'
    Assert-True ((Get-Content (Join-Path $rollbackDest 'speedqx.exe') -Raw) -eq 'old-speedqx') 'speedqx.exe was not restored after copy failure'

    # A fresh destination has no retired pair to restore. If the second copy
    # fails, the first staged binary must still be removed.
    $cleanRollbackRoot = Join-Path $temp 'clean-rollback'
    $cleanRollbackDest = Join-Path $cleanRollbackRoot 'dest'
    $cleanRollbackSource = Join-Path $cleanRollbackRoot 'source'
    $cleanRollbackLib = Join-Path $cleanRollbackRoot 'lib'
    New-Item -ItemType Directory -Force -Path $cleanRollbackDest, $cleanRollbackSource, $cleanRollbackLib | Out-Null
    Copy-Item -LiteralPath "$env:SystemRoot\System32\where.exe" -Destination (Join-Path $cleanRollbackSource 'nd300.exe')
    $cleanRollbackFailed = $false
    try {
        Invoke-Installer @{
            bin_paths = @(
                (Join-Path $cleanRollbackSource 'nd300.exe'),
                (Join-Path $cleanRollbackSource 'missing-speedqx.exe')
            )
            lib_paths = @()
        } $cleanRollbackDest $cleanRollbackLib
    } catch {
        $cleanRollbackFailed = $true
    }
    Assert-True $cleanRollbackFailed 'Injected clean-destination copy failure unexpectedly succeeded'
    Assert-True (-not (Test-Path -LiteralPath (Join-Path $cleanRollbackDest 'nd300.exe'))) 'A staged nd300.exe remained after clean-destination rollback'
    Assert-True (-not (Test-Path -LiteralPath (Join-Path $cleanRollbackDest 'speedqx.exe'))) 'A staged speedqx.exe remained after clean-destination rollback'

    # Failure during the second retirement must roll back the already-retired
    # first binary as well.
    $partialRoot = Join-Path $temp 'partial-retirement'
    $partialDest = Join-Path $partialRoot 'dest'
    $partialSource = Join-Path $partialRoot 'source'
    $partialLib = Join-Path $partialRoot 'lib'
    New-Item -ItemType Directory -Force -Path $partialDest, $partialSource, $partialLib | Out-Null
    [System.IO.File]::WriteAllText((Join-Path $partialDest 'nd300.exe'), 'old-nd300')
    [System.IO.File]::WriteAllText((Join-Path $partialDest 'speedqx.exe'), 'old-speedqx')
    $staleProcess = Start-LockedImage (Join-Path $partialDest 'speedqx.update-old-9.8.7.exe')
    Copy-Item -LiteralPath "$env:SystemRoot\System32\where.exe" -Destination (Join-Path $partialSource 'nd300.exe')
    Copy-Item -LiteralPath "$env:SystemRoot\System32\whoami.exe" -Destination (Join-Path $partialSource 'speedqx.exe')
    $retirementFailed = $false
    try {
        Invoke-Installer @{
            bin_paths = @(
                (Join-Path $partialSource 'nd300.exe'),
                (Join-Path $partialSource 'speedqx.exe')
            )
            lib_paths = @()
        } $partialDest $partialLib
    } catch {
        $retirementFailed = $true
    }
    Assert-True $retirementFailed 'Locked stale retirement fixture unexpectedly succeeded'
    Assert-True ((Get-Content (Join-Path $partialDest 'nd300.exe') -Raw) -eq 'old-nd300') 'First binary was stranded after second retirement failed'
    Assert-True ((Get-Content (Join-Path $partialDest 'speedqx.exe') -Raw) -eq 'old-speedqx') 'Second original binary changed after retirement failure'

    Write-Host 'cargo-dist Windows installer hardening fixtures: PASS'
} finally {
    Stop-TestProcess $liveProcess
    Stop-TestProcess $staleProcess
    Remove-Item -LiteralPath $temp -Recurse -Force -ErrorAction SilentlyContinue
}
