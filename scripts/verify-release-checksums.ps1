[CmdletBinding()]
param(
    [Parameter(Mandatory)]
    [ValidateNotNullOrEmpty()]
    [string]$Directory
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

$releaseDirectory = (Resolve-Path -LiteralPath $Directory -ErrorAction Stop).Path
if (-not (Test-Path -LiteralPath $releaseDirectory -PathType Container)) {
    throw "Release directory is not a directory: $releaseDirectory"
}

function Assert-SafeAssetName {
    param(
        [Parameter(Mandatory)]
        [string]$Name,

        [Parameter(Mandatory)]
        [string]$Context
    )

    if ([System.IO.Path]::GetFileName($Name) -ne $Name) {
        throw "$Context names a non-local target: $Name"
    }
}

$sidecars = @(Get-ChildItem -LiteralPath $releaseDirectory -Filter '*.sha256' -File)
if ($sidecars.Count -eq 0) {
    throw 'Release contains no SHA-256 sidecars'
}

foreach ($sidecar in $sidecars) {
    $line = [System.IO.File]::ReadAllText($sidecar.FullName).Trim()
    if ($line -notmatch '^([0-9a-fA-F]{64})\s+\*?(.+)$') {
        throw "Malformed SHA-256 sidecar: $($sidecar.Name)"
    }

    $expectedHash = $Matches[1].ToLowerInvariant()
    $targetName = $Matches[2]
    Assert-SafeAssetName -Name $targetName -Context $sidecar.Name
    if ($targetName -ne $sidecar.Name.Substring(0, $sidecar.Name.Length - 7)) {
        throw "Sidecar $($sidecar.Name) names unexpected target $targetName"
    }

    $targetPath = Join-Path $releaseDirectory $targetName
    if (-not (Test-Path -LiteralPath $targetPath -PathType Leaf)) {
        throw "Sidecar target is missing: $targetName"
    }
    $actualHash = (Get-FileHash -LiteralPath $targetPath -Algorithm SHA256).Hash.ToLowerInvariant()
    if ($actualHash -ne $expectedHash) {
        throw "SHA-256 mismatch for $targetName"
    }
}

$aggregatePath = Join-Path $releaseDirectory 'sha256.sum'
if (-not (Test-Path -LiteralPath $aggregatePath -PathType Leaf)) {
    throw 'Release is missing sha256.sum'
}
$aggregateLines = @(
    [System.IO.File]::ReadAllLines($aggregatePath) |
        Where-Object { -not [string]::IsNullOrWhiteSpace($_) }
)
if ($aggregateLines.Count -eq 0) {
    throw 'sha256.sum contains no checksum entries'
}

foreach ($line in $aggregateLines) {
    if ($line -notmatch '^([0-9a-fA-F]{64})\s+\*?(.+)$') {
        throw "Malformed sha256.sum entry: $line"
    }

    $expectedHash = $Matches[1].ToLowerInvariant()
    $targetName = $Matches[2]
    Assert-SafeAssetName -Name $targetName -Context 'sha256.sum'
    $targetPath = Join-Path $releaseDirectory $targetName
    if (-not (Test-Path -LiteralPath $targetPath -PathType Leaf)) {
        throw "sha256.sum target is missing: $targetName"
    }
    $actualHash = (Get-FileHash -LiteralPath $targetPath -Algorithm SHA256).Hash.ToLowerInvariant()
    if ($actualHash -ne $expectedHash) {
        throw "sha256.sum mismatch for $targetName"
    }
}

Write-Host "Verified $($sidecars.Count) SHA-256 sidecars and $($aggregateLines.Count) nonblank sha256.sum entries."
