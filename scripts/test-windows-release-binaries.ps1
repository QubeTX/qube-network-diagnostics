Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
. (Join-Path $PSScriptRoot 'prepare-windows-release-binaries.ps1')
$repository = 'QubeTX/qube-network-diagnostics'
$tag = 'v9.8.7'
$source = 'a' * 40
$script:resolved = $source
$script:attestations = 0
$script:rejectAttestation = 0
$script:executions = 0
$script:badVersion = $false
$script:wrongSigner = $false

# Stub only the external trust service and CLI payloads. ZIP parsing, checksum
# checking, extraction bounds, atomic publication, and failure cleanup are real.
function gh {
    param([Parameter(ValueFromRemainingArguments)][object[]]$Arguments)
    $global:LASTEXITCODE = 0
    if ($Arguments[0] -eq 'api') { return $script:resolved }
    if ($Arguments[0] -ne 'attestation' -or $Arguments[1] -ne 'verify') { throw 'Unexpected gh invocation.' }
    foreach ($pair in @(
        @('--repo', $repository), @('--signer-repo', $repository),
        @('--source-digest', $source), @('--source-ref', "refs/tags/$tag")
    )) {
        $index = [Array]::IndexOf($Arguments, $pair[0])
        if ($index -lt 0 -or $Arguments[$index + 1] -ne $pair[1]) { throw "Missing attestation constraint: $($pair[0])" }
    }
    $script:attestations++
    if ($script:attestations -eq $script:rejectAttestation) { $global:LASTEXITCODE = 1 }
    $workflow = if ($script:wrongSigner) { 'unrelated.yml' } else { 'release.yml' }
    return @{ verificationResult=@{ signature=@{ certificate=@{
        buildSignerURI="https://github.com/$repository/.github/workflows/$workflow@refs/tags/$tag"
    } } } } | ConvertTo-Json -Depth 5 -Compress
}
function Get-WindowsReleaseBinaryVersion {
    param([string]$Path)
    if ($script:attestations -ne 2) { throw 'Binary executed before both attestations passed.' }
    $script:executions++
    $name = [IO.Path]::GetFileNameWithoutExtension($Path)
    if ($script:badVersion -and $name -eq 'speedqx') { return 'speedqx 0.0.0' }
    return "$name 9.8.7"
}

$tempRoot = [IO.Path]::GetFullPath([IO.Path]::GetTempPath()).TrimEnd('\', '/')
$fixture = Join-Path $tempRoot ('nd300-release-fixture-' + [guid]::NewGuid().ToString('N'))
$archive = Join-Path $fixture 'nd300-x86_64-pc-windows-msvc.zip'
$checksum = "$archive.sha256"

function New-Archive {
    param([string[]]$Names = @('nd300.exe', 'speedqx.exe'), [switch]$Symlink)
    $stream = [IO.File]::Open($archive, [IO.FileMode]::Create)
    $zip = [IO.Compression.ZipArchive]::new($stream, [IO.Compression.ZipArchiveMode]::Create)
    try {
        foreach ($name in $Names) {
            $entry = $zip.CreateEntry($name)
            if ($Symlink -and $name -eq 'nd300.exe') { $entry.ExternalAttributes = 0xa000 -shl 16 }
            $writer = [IO.StreamWriter]::new($entry.Open())
            try { $writer.Write('fixture binary or documentation') } finally { $writer.Dispose() }
        }
    }
    finally { $zip.Dispose(); $stream.Dispose() }
    Update-Checksum
    $script:attestations = 0
    $script:executions = 0
    $script:rejectAttestation = 0
    $script:badVersion = $false
    $script:wrongSigner = $false
}
function Update-Checksum {
    $hash = (Get-FileHash -LiteralPath $archive -Algorithm SHA256).Hash
    [IO.File]::WriteAllText($checksum, "$hash *nd300-x86_64-pc-windows-msvc.zip")
}
function Assert-Rejected {
    param([string]$Pattern)
    $destination = Join-Path $fixture ('rejected-' + [guid]::NewGuid().ToString('N'))
    $rejected = $false
    try { Expand-VerifiedWindowsReleasePair $archive $checksum $repository $tag $source $destination | Out-Null }
    catch {
        if ($_.Exception.Message -notmatch $Pattern) { throw }
        $rejected = $true
    }
    if (-not $rejected) { throw "Expected rejection matching $Pattern" }
    if (Test-Path -LiteralPath $destination) { throw 'A rejected pair became visible.' }
    if (@(Get-ChildItem -LiteralPath $fixture -Directory -Filter 'nd300-verified-pair-*').Count) { throw 'Rejected extraction left staging files.' }
}

try {
    New-Item -ItemType Directory -Path $fixture | Out-Null
    Assert-WindowsReleaseIdentity $repository $tag $source
    $script:resolved = 'b' * 40
    try { Assert-WindowsReleaseIdentity $repository $tag $source; throw 'Wrong repository tag source was accepted.' }
    catch { if ($_.Exception.Message -notmatch 'does not resolve') { throw } }
    $script:resolved = $source

    New-Archive -Names @('README.md', 'man/nd300.1', 'nd300.exe', 'speedqx.exe')
    $destination = Join-Path $fixture 'accepted'
    Expand-VerifiedWindowsReleasePair $archive $checksum $repository $tag $source $destination | Out-Null
    if (@(Get-ChildItem -LiteralPath $destination -File).Count -ne 2 -or $script:executions -ne 2) {
        throw 'Valid archive did not publish only the verified pair.'
    }
    try {
        Expand-VerifiedWindowsReleasePair $archive $checksum $repository $tag $source $destination | Out-Null
        throw 'Existing binary destination was overwritten.'
    }
    catch { if ($_.Exception.Message -notmatch 'already exists') { throw } }

    New-Archive
    [IO.File]::AppendAllText($archive, 'replaced bytes')
    Assert-Rejected 'SHA-256 mismatch'
    if ($script:attestations -or $script:executions) { throw 'Corrupt archive crossed the integrity boundary.' }
    New-Archive
    [IO.File]::WriteAllText($checksum, ('a' * 64) + ' *../outside.zip')
    Assert-Rejected 'Malformed.*checksum'
    foreach ($failure in @(1, 2)) {
        New-Archive
        $script:rejectAttestation = $failure
        Assert-Rejected 'attestation rejected'
        if ($script:executions) { throw 'Unattested archive was executed.' }
    }
    New-Archive
    $script:wrongSigner = $true
    Assert-Rejected 'expected tag release workflow'
    if ($script:executions) { throw 'Unrelated workflow archive was executed.' }
    foreach ($unsafe in @('../outside', '/absolute', 'C:/drive', 'folder\outside', 'folder/../outside', 'folder//outside', 'ND300.EXE')) {
        New-Archive -Names @('nd300.exe', 'speedqx.exe', $unsafe)
        Assert-Rejected 'Unsafe or duplicate'
        if ($script:executions) { throw 'Unsafe archive was executed.' }
    }
    New-Archive -Names @('nd300.exe')
    Assert-Rejected 'intended binary pair'
    New-Archive -Symlink
    Assert-Rejected 'Unsafe or duplicate'
    New-Archive
    $bytes = [IO.File]::ReadAllBytes($archive)
    for ($i = 0; $i -lt $bytes.Length - 28; $i++) {
        if ([BitConverter]::ToUInt32($bytes, $i) -eq 0x02014b50) {
            [BitConverter]::GetBytes([uint32](129MB)).CopyTo($bytes, $i + 24)
            break
        }
    }
    [IO.File]::WriteAllBytes($archive, $bytes)
    Update-Checksum
    Assert-Rejected 'extraction bounds'
    if ($script:executions) { throw 'Oversized archive was executed.' }
    New-Archive
    $script:badVersion = $true
    Assert-Rejected 'version does not match'
}
finally {
    if ([IO.Path]::GetDirectoryName([IO.Path]::GetFullPath($fixture)) -ne $tempRoot -or
        [IO.Path]::GetFileName($fixture) -notlike 'nd300-release-fixture-*') { throw 'Fixture cleanup escaped the temporary root.' }
    if (Test-Path -LiteralPath $fixture) { Remove-Item -LiteralPath $fixture -Recurse -Force }
}
Write-Host 'Verified Windows release archive fixtures passed.'
