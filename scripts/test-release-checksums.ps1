Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

$verifier = Join-Path $PSScriptRoot 'verify-release-checksums.ps1'
$testDirectory = Join-Path ([System.IO.Path]::GetTempPath()) "nd300-checksum-fixture-$([guid]::NewGuid().ToString('N'))"

function Invoke-ExpectedFailure {
    param(
        [Parameter(Mandatory)]
        [string]$MessagePattern
    )

    $failed = $false
    try {
        & $verifier -Directory $testDirectory *> $null
    }
    catch {
        $failed = $true
        if ($_.Exception.Message -notmatch $MessagePattern) {
            throw "Expected failure matching '$MessagePattern', got: $($_.Exception.Message)"
        }
    }
    if (-not $failed) {
        throw "Expected checksum verifier failure matching '$MessagePattern'"
    }
}

try {
    New-Item -ItemType Directory -Path $testDirectory -ErrorAction Stop | Out-Null
    $artifactName = 'artifact.bin'
    $artifactPath = Join-Path $testDirectory $artifactName
    [System.IO.File]::WriteAllText($artifactPath, 'trusted fixture')
    $hash = (Get-FileHash -LiteralPath $artifactPath -Algorithm SHA256).Hash.ToLowerInvariant()
    $validLine = "$hash *$artifactName"
    [System.IO.File]::WriteAllText("$artifactPath.sha256", "$validLine`n")

    # cargo-dist currently emits an extra trailing blank line. Whitespace-only
    # separators are harmless, while every nonblank line remains strict.
    [System.IO.File]::WriteAllText((Join-Path $testDirectory 'sha256.sum'), "$validLine`n`n`t`n")
    & $verifier -Directory $testDirectory | Out-Null

    [System.IO.File]::WriteAllText(
        (Join-Path $testDirectory 'sha256.sum'),
        "$validLine`nnot-a-checksum`n"
    )
    Invoke-ExpectedFailure -MessagePattern 'Malformed sha256\.sum entry'

    [System.IO.File]::WriteAllText((Join-Path $testDirectory 'sha256.sum'), "`n`t`n")
    Invoke-ExpectedFailure -MessagePattern 'contains no checksum entries'

    [System.IO.File]::WriteAllText((Join-Path $testDirectory 'sha256.sum'), "$validLine`n")
    [System.IO.File]::WriteAllText($artifactPath, 'corrupted fixture')
    Invoke-ExpectedFailure -MessagePattern 'SHA-256 mismatch'
}
finally {
    if (Test-Path -LiteralPath $testDirectory) {
        Remove-Item -LiteralPath $testDirectory -Recurse -Force
    }
}

Write-Host 'Release checksum verifier fixtures passed.'
