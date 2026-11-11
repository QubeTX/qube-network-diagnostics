<#
.SYNOPSIS
  Build all four ND-300 Windows installers locally and report exit codes.

.DESCRIPTION
  Reproduces — on the developer's machine, BEFORE a tag-push release — exactly
  what .github/workflows/windows-installers.yml does in CI, so installer-build
  bugs (e.g. WiX CNDL0104 from a `--` in an XML comment, or an Inno invalid-
  constant error) are caught locally instead of at release time.

  Steps:
    1. Ensure WiX 3.11 binaries (candle.exe / light.exe) are available.
       Downloads wix311-binaries.zip from the WiX Toolset GitHub release into a
       local cache if not already present / not on PATH. Verifies the pinned
       ZIP hash before every extraction, including reuse of a cached archive.
    2. Ensure Inno Setup 6 (iscc.exe) is available. Downloads the pinned
       publisher installer, verifies its SHA-256, then installs it
       (/CURRENTUSER /VERYSILENT) if missing.
    3. cargo build --release (produces target\release\nd300.exe + speedqx.exe).
    4. Build the 4 installers:
         - Global MSI       : wix\main.wxs            (candle + light)
         - Corporate MSI    : wix-corporate\corporate.wxs
                              (candle + light -sice:ICE38 -sice:ICE64 -sice:ICE91)
         - Global EXE       : inno\global.iss         (iscc /DMyAppVersion=...)
         - Corporate EXE    : inno\corporate.iss      (iscc /DMyAppVersion=...)
    5. Print a summary table of exit codes; exit non-zero if any installer failed.

  Version is read from Cargo.toml ([package] version) unless -Version is passed.

  PowerShell arg-quoting gotcha (the reason every -d.../ /D... arg below is a
  single quoted string): without quotes PowerShell tokenizes `-dVersion=3.3.0`
  as `-dVersion=3` followed by `.3.0`, and `/DMyAppVersion=3.3.0` similarly, so
  candle/iscc see a stray `.3.0` as a source filename (CNDL0103 / iscc error).
  Quoting forces the whole assignment to be one literal token.

.PARAMETER RepoRoot
  Path to the repo root. Defaults to two levels up from this script
  (.claude\skills\validate-installers\scripts\ -> repo root).

.PARAMETER Version
  Override the version passed to the installers. Defaults to Cargo.toml's version.

.PARAMETER SkipBuild
  Skip `cargo build --release` (use existing target\release binaries).
#>
[CmdletBinding()]
param(
    [string]$RepoRoot,
    [string]$Version,
    [switch]$SkipBuild
)

$ErrorActionPreference = 'Stop'

# --- Resolve repo root --------------------------------------------------------
if (-not $RepoRoot) {
    $RepoRoot = (Resolve-Path (Join-Path $PSScriptRoot '..\..\..\..')).Path
}
$RepoRoot = (Resolve-Path $RepoRoot).Path
Write-Host "Repo root: $RepoRoot"

if (-not (Test-Path (Join-Path $RepoRoot 'Cargo.toml'))) {
    throw "Cargo.toml not found under $RepoRoot — is -RepoRoot correct?"
}

# --- Resolve version ----------------------------------------------------------
if (-not $Version) {
    $cargoToml = Get-Content (Join-Path $RepoRoot 'Cargo.toml')
    $inPackage = $false
    foreach ($line in $cargoToml) {
        if ($line -match '^\s*\[package\]') { $inPackage = $true; continue }
        if ($line -match '^\s*\[') { $inPackage = $false }
        if ($inPackage -and $line -match '^\s*version\s*=\s*"([^"]+)"') {
            $Version = $Matches[1]; break
        }
    }
}
if (-not $Version) { throw "Could not determine version from Cargo.toml; pass -Version." }
Write-Host "Version: $Version"

$cacheDir = Join-Path $env:LOCALAPPDATA 'nd300-installer-tools'
New-Item -ItemType Directory -Force -Path $cacheDir | Out-Null
if ((Get-Item -LiteralPath $cacheDir -Force).Attributes -band [IO.FileAttributes]::ReparsePoint) {
    throw 'Refusing a redirected installer-tool cache.'
}

# Publisher release inputs, verified against their official release assets:
# https://github.com/wixtoolset/wix3/releases/tag/wix3112rtm
# https://github.com/jrsoftware/issrc/releases/tag/is-6_7_4
$WixZipUrl = 'https://github.com/wixtoolset/wix3/releases/download/wix3112rtm/wix311-binaries.zip'
$WixZipSha256 = '2c1888d5d1dba377fc7fa14444cf556963747ff9a0a289a3599cf09da03b9e2e'
$InnoSetupUrl = 'https://github.com/jrsoftware/issrc/releases/download/is-6_7_4/innosetup-6.7.4.exe'
$InnoSetupSha256 = 'a549929319f9120716677c685535ca4671e4b937b5fdc87e2585f32d8eb30e8c'

function Assert-FileSha256 {
    param(
        [Parameter(Mandatory=$true)][string]$Path,
        [Parameter(Mandatory=$true)][string]$ExpectedSha256,
        [Parameter(Mandatory=$true)][string]$Label
    )

    $actual = (Get-FileHash -LiteralPath $Path -Algorithm SHA256).Hash.ToLowerInvariant()
    if ($actual -ne $ExpectedSha256.ToLowerInvariant()) {
        throw "$Label SHA-256 mismatch. Refusing to extract or execute downloaded tool."
    }
}

function Expand-VerifiedWix {
    param([Parameter(Mandatory=$true)][string]$Archive)

    Assert-FileSha256 -Path $Archive -ExpectedSha256 $WixZipSha256 -Label 'WiX 3.11 binaries ZIP'
    # A marker cannot authenticate extracted executables. Recreate only our
    # dedicated cache subdirectory from the verified archive on every use.
    $destination = [IO.Path]::GetFullPath((Join-Path $cacheDir 'wix3112rtm'))
    $cacheRoot = (Resolve-Path -LiteralPath $cacheDir).Path
    if ([IO.Path]::GetDirectoryName($destination) -ne $cacheRoot -or
        [IO.Path]::GetFileName($destination) -ne 'wix3112rtm') {
        throw 'WiX extraction path escaped the managed cache.'
    }
    if (Test-Path -LiteralPath $destination) {
        $item = Get-Item -LiteralPath $destination -Force
        if ($item.Attributes -band [IO.FileAttributes]::ReparsePoint) {
            throw 'Refusing a redirected WiX extraction directory.'
        }
        Remove-Item -LiteralPath $destination -Recurse -Force
    }
    Expand-Archive -LiteralPath $Archive -DestinationPath $destination
    return $destination
}

function Install-VerifiedInno {
    param([Parameter(Mandatory=$true)][string]$Installer)

    Assert-FileSha256 -Path $Installer -ExpectedSha256 $InnoSetupSha256 -Label 'Inno Setup installer'
    return Start-Process -FilePath $Installer -ArgumentList '/CURRENTUSER','/VERYSILENT','/SUPPRESSMSGBOXES','/NORESTART' -WindowStyle Hidden -Wait -PassThru
}

# --- Ensure WiX 3.11 (candle/light) ------------------------------------------
function Get-WixBinDir {
    # Installed tools outside our cache may come from $env:WIX\bin or PATH.
    $candidates = @()
    if ($env:WIX) { $candidates += Join-Path $env:WIX 'bin' }
    $candle = Get-Command candle.exe -ErrorAction SilentlyContinue
    if ($candle) { $candidates += Split-Path $candle.Source }
    $cacheRoot = [IO.Path]::GetFullPath($cacheDir).TrimEnd('\', '/')
    $cachePrefix = $cacheRoot + [IO.Path]::DirectorySeparatorChar
    foreach ($candidate in $candidates) {
        $absolute = [IO.Path]::GetFullPath($candidate).TrimEnd('\', '/')
        # Adding an old flat cache or our versioned extraction to PATH/WIX
        # must not turn cached bytes into a trusted installed-tool override.
        if ($absolute -eq $cacheRoot -or
            $absolute.StartsWith($cachePrefix, [StringComparison]::OrdinalIgnoreCase)) {
            continue
        }
        if ((Test-Path (Join-Path $absolute 'candle.exe')) -and
            (Test-Path (Join-Path $absolute 'light.exe'))) {
            return $absolute
        }
    }
    # Managed extracted caches are never returned here without ZIP verification.
    return $null
}

$wixBin = Get-WixBinDir
if (-not $wixBin) {
    Write-Host "WiX 3.11 not found — downloading wix311-binaries.zip..."
    $zip = Join-Path $cacheDir 'wix311-binaries.zip'
    if (-not (Test-Path -LiteralPath $zip)) {
        Invoke-WebRequest -Uri $WixZipUrl -OutFile $zip -UseBasicParsing
    }
    $wixBin = Expand-VerifiedWix -Archive $zip
}
if (-not $wixBin -or -not (Test-Path (Join-Path $wixBin 'candle.exe')) -or
    -not (Test-Path (Join-Path $wixBin 'light.exe'))) {
    throw "Failed to obtain WiX 3.11 candle/light binaries."
}
$candle = Join-Path $wixBin 'candle.exe'
$light  = Join-Path $wixBin 'light.exe'
Write-Host "WiX bin: $wixBin"

# --- Ensure Inno Setup 6 (iscc) ----------------------------------------------
function Get-Iscc {
    $iscc = Get-Command iscc.exe -ErrorAction SilentlyContinue
    if ($iscc) { return $iscc.Source }
    foreach ($p in @(
        'C:\Program Files (x86)\Inno Setup 6\iscc.exe',
        'C:\Program Files\Inno Setup 6\iscc.exe',
        (Join-Path $env:LOCALAPPDATA 'Programs\Inno Setup 6\iscc.exe')
    )) { if (Test-Path $p) { return $p } }
    return $null
}

$iscc = Get-Iscc
if (-not $iscc) {
    Write-Host "Inno Setup 6 not found — installing (/CURRENTUSER /VERYSILENT)..."
    $innoExe = Join-Path $cacheDir 'innosetup-6.7.4.exe'
    Invoke-WebRequest -Uri $InnoSetupUrl -OutFile $innoExe -UseBasicParsing
    $p = Install-VerifiedInno -Installer $innoExe
    if ($p.ExitCode -ne 0) { throw "Inno Setup install failed with exit $($p.ExitCode)." }
    $iscc = Get-Iscc
}
if (-not $iscc) { throw "Failed to obtain Inno Setup 6 iscc.exe." }
Write-Host "iscc: $iscc"

# --- Build release binaries ---------------------------------------------------
if (-not $SkipBuild) {
    Write-Host "`ncargo build --release ..."
    Push-Location $RepoRoot
    try {
        & cargo build --release
        if ($LASTEXITCODE -ne 0) { throw "cargo build --release failed (exit $LASTEXITCODE)." }
    } finally { Pop-Location }
}
foreach ($exe in @('nd300.exe','speedqx.exe')) {
    $path = Join-Path $RepoRoot "target\release\$exe"
    if (-not (Test-Path $path)) { throw "Missing $path — release build did not produce both binaries." }
}

# --- Build the four installers ------------------------------------------------
$results = [ordered]@{}
$wixObjDir = Join-Path $RepoRoot 'target\wix'
New-Item -ItemType Directory -Force -Path $wixObjDir | Out-Null
$binDir = 'target\release'   # relative; candle is invoked from RepoRoot

Push-Location $RepoRoot
try {
    # 1. Global MSI (wix\main.wxs)
    Write-Host "`n[1/4] Global MSI (wix\main.wxs)"
    & $candle -arch x64 "-dVersion=$Version" "-dCargoTargetBinDir=$binDir" `
        -ext WixUIExtension -out "$wixObjDir\main.wixobj" wix\main.wxs
    $c1 = $LASTEXITCODE
    if ($c1 -eq 0) {
        & $light -ext WixUIExtension `
            -out "$wixObjDir\nd300-x86_64-pc-windows-msvc.msi" "$wixObjDir\main.wixobj"
        $c1 = $LASTEXITCODE
    }
    $results['Global MSI (wix/main.wxs)'] = $c1

    # 2. Corporate MSI (wix-corporate\corporate.wxs) — ICE suppressions at light.
    Write-Host "`n[2/4] Corporate MSI (wix-corporate\corporate.wxs)"
    & $candle -arch x64 "-dVersion=$Version" "-dCargoTargetBinDir=$binDir" `
        -ext WixUIExtension -out "$wixObjDir\corporate.wixobj" wix-corporate\corporate.wxs
    $c2 = $LASTEXITCODE
    if ($c2 -eq 0) {
        & $light -sice:ICE38 -sice:ICE64 -sice:ICE91 -ext WixUIExtension `
            -out "$wixObjDir\nd300-x86_64-pc-windows-msvc-corporate.msi" "$wixObjDir\corporate.wixobj"
        $c2 = $LASTEXITCODE
    }
    $results['Corporate MSI (wix-corporate/corporate.wxs)'] = $c2

    # 3. Global EXE (inno\global.iss)
    Write-Host "`n[3/4] Global EXE (inno\global.iss)"
    & $iscc "/DMyAppVersion=$Version" inno\global.iss
    $results['Global EXE (inno/global.iss)'] = $LASTEXITCODE

    # 4. Corporate EXE (inno\corporate.iss)
    Write-Host "`n[4/4] Corporate EXE (inno\corporate.iss)"
    & $iscc "/DMyAppVersion=$Version" inno\corporate.iss
    $results['Corporate EXE (inno/corporate.iss)'] = $LASTEXITCODE
} finally { Pop-Location }

# --- Summary ------------------------------------------------------------------
Write-Host "`n==================== INSTALLER BUILD SUMMARY ===================="
$failed = 0
foreach ($k in $results.Keys) {
    $code = $results[$k]
    $status = if ($code -eq 0) { 'OK  ' } else { 'FAIL'; }
    if ($code -ne 0) { $failed++ }
    Write-Host ("  [{0}] exit={1,-4} {2}" -f $status, $code, $k)
}
Write-Host "================================================================"
if ($failed -gt 0) {
    Write-Host "$failed installer(s) FAILED. Do NOT tag-push the release until fixed." -ForegroundColor Red
    exit 1
}
Write-Host "All 4 installers built successfully." -ForegroundColor Green
exit 0
