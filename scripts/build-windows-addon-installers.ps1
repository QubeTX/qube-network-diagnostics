[CmdletBinding()]
param(
    [Parameter(Mandatory)][string]$Version,
    [string]$RepoRoot = (Get-Location).Path
)
Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
if ($Version -notmatch '^[0-9]+\.[0-9]+\.[0-9]+(?:[-+][0-9A-Za-z.-]+)?$') { throw 'Invalid installer version.' }
$root = (Resolve-Path -LiteralPath $RepoRoot).Path
foreach ($binary in @('nd300.exe', 'speedqx.exe')) {
    if (-not (Test-Path -LiteralPath (Join-Path $root "target/release/$binary") -PathType Leaf)) {
        throw "Missing verified release binary: $binary"
    }
}
$candle = Join-Path $env:WIX 'bin/candle.exe'
$light = Join-Path $env:WIX 'bin/light.exe'
if (-not (Test-Path -LiteralPath $candle) -or -not (Test-Path -LiteralPath $light)) { throw 'WiX candle/light are unavailable.' }
$iscc = 'C:\Program Files (x86)\Inno Setup 6\iscc.exe'
if (-not (Test-Path -LiteralPath $iscc)) {
    choco install innosetup -y --no-progress --no-color
    if ($LASTEXITCODE -ne 0 -or -not (Test-Path -LiteralPath $iscc)) { throw 'Inno Setup 6 is unavailable.' }
}
Push-Location $root
try {
    New-Item -ItemType Directory -Force -Path target/wix | Out-Null
    # Keep the existing per-user ICE suppressions and quoted version defines.
    & $candle -arch x64 "-dVersion=$Version" '-dCargoTargetBinDir=target\release' `
        -out target/wix/corporate.wixobj wix-corporate/corporate.wxs
    if ($LASTEXITCODE -ne 0) { throw "Corporate candle failed: $LASTEXITCODE" }
    & $light -sice:ICE38 -sice:ICE64 -sice:ICE91 -ext WixUIExtension `
        -out target/wix/nd300-x86_64-pc-windows-msvc-corporate.msi target/wix/corporate.wixobj
    if ($LASTEXITCODE -ne 0) { throw "Corporate light failed: $LASTEXITCODE" }
    & $iscc "/DMyAppVersion=$Version" inno/global.iss
    if ($LASTEXITCODE -ne 0) { throw "Global Inno build failed: $LASTEXITCODE" }
    & $iscc "/DMyAppVersion=$Version" inno/corporate.iss
    if ($LASTEXITCODE -ne 0) { throw "Corporate Inno build failed: $LASTEXITCODE" }
    foreach ($asset in @(
        'target/wix/nd300-x86_64-pc-windows-msvc-corporate.msi',
        'inno/Output/nd300-x86_64-pc-windows-msvc-setup.exe',
        'inno/Output/nd300-x86_64-pc-windows-msvc-corporate-setup.exe'
    )) {
        if (-not (Test-Path -LiteralPath $asset -PathType Leaf)) { throw "Installer output missing: $asset" }
        $hash = (Get-FileHash -LiteralPath $asset -Algorithm SHA256).Hash.ToLowerInvariant()
        [IO.File]::WriteAllText((Join-Path $root "$asset.sha256"), "$hash *$([IO.Path]::GetFileName($asset))")
    }
}
finally { Pop-Location }
Write-Host 'Built all three installer add-ons and checksum sidecars from the verified release pair.'
