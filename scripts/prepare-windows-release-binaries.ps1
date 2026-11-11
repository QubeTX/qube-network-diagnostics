[CmdletBinding()]
param(
    [string]$Repository,
    [string]$Tag,
    [string]$SourceSha,
    [string]$RepoRoot = (Get-Location).Path
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

function Assert-WindowsReleaseIdentity {
    param([string]$Repository, [string]$Tag, [string]$SourceSha)
    if ($Repository -notmatch '^[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+$' -or
        $Tag -notmatch '^v[0-9]+\.[0-9]+\.[0-9]+(?:[-+][0-9A-Za-z.-]+)?$' -or
        $SourceSha -notmatch '^[a-fA-F0-9]{40}$') {
        throw 'Invalid repository, release tag, or source SHA.'
    }
    # Resolve the repository's actual tag through GitHub; a caller's source
    # input alone is never proof that these are the intended release bytes.
    $resolved = gh api "repos/$Repository/commits/$Tag" --jq '.sha'
    if ($LASTEXITCODE -ne 0 -or "$resolved".Trim() -ne $SourceSha) {
        throw 'Repository release tag does not resolve to the expected source SHA.'
    }
}

function Assert-WindowsReleaseAttestation {
    param([string]$Path, [string]$Repository, [string]$Tag, [string]$SourceSha)
    $verified = gh attestation verify $Path --repo $Repository --signer-repo $Repository `
        --source-digest $SourceSha --source-ref "refs/tags/$Tag" --format json | ConvertFrom-Json
    if ($LASTEXITCODE -ne 0) { throw "Release attestation rejected: $([IO.Path]::GetFileName($Path))" }
    # gh treats --signer-repo and --signer-workflow as mutually exclusive.
    # Verify the repository cryptographically, then constrain its verified signer.
    $expectedSigner = "https://github.com/$Repository/.github/workflows/release.yml@refs/tags/$Tag"
    if (-not @($verified | Where-Object {
        $_.verificationResult.signature.certificate.buildSignerURI -ceq $expectedSigner
    }).Count) { throw 'Release attestation is not from the expected tag release workflow.' }
}

function Get-WindowsReleaseBinaryVersion {
    param([string]$Path)
    $start = [Diagnostics.ProcessStartInfo]::new($Path, '--version')
    $start.UseShellExecute = $false
    $start.CreateNoWindow = $true
    $start.RedirectStandardOutput = $true
    $start.RedirectStandardError = $true
    $process = [Diagnostics.Process]::new()
    $process.StartInfo = $start
    try {
        if (-not $process.Start()) { throw 'Could not start verified release binary.' }
        $stdout = $process.StandardOutput.ReadToEndAsync()
        $stderr = $process.StandardError.ReadToEndAsync()
        if (-not $process.WaitForExit(10000)) {
            $process.Kill($true)
            $process.WaitForExit()
            throw 'Verified release binary version check timed out.'
        }
        $output = $stdout.GetAwaiter().GetResult().Trim()
        $errorOutput = $stderr.GetAwaiter().GetResult()
        if ($process.ExitCode -ne 0 -or $output.Length -gt 128 -or $errorOutput.Length -gt 128) {
            throw 'Verified release binary version check failed.'
        }
        return $output
    }
    finally { $process.Dispose() }
}

function Expand-VerifiedWindowsReleasePair {
    param(
        [string]$Archive, [string]$Checksum, [string]$Repository,
        [string]$Tag, [string]$SourceSha, [string]$Destination
    )
    $assetName = 'nd300-x86_64-pc-windows-msvc.zip'
    if ((Get-Item -LiteralPath $Archive).Length -gt 256MB -or
        (Get-Item -LiteralPath $Checksum).Length -gt 512) {
        throw 'Release archive or checksum exceeds its size limit.'
    }
    $line = [IO.File]::ReadAllText($Checksum).Trim()
    if ($line -notmatch '^([a-fA-F0-9]{64})[ \t]+\*?nd300-x86_64-pc-windows-msvc\.zip$') {
        throw 'Malformed release archive checksum.'
    }
    $expectedHash = $Matches[1]
    if ((Get-FileHash -LiteralPath $Archive -Algorithm SHA256).Hash -ne $expectedHash) {
        throw 'Release archive SHA-256 mismatch.'
    }
    Assert-WindowsReleaseAttestation $Archive $Repository $Tag $SourceSha
    Assert-WindowsReleaseAttestation $Checksum $Repository $Tag $SourceSha

    $destinationPath = [IO.Path]::GetFullPath($Destination)
    if (Test-Path -LiteralPath $destinationPath) { throw 'Release binary destination already exists.' }
    $parent = [IO.Path]::GetDirectoryName($destinationPath)
    New-Item -ItemType Directory -Path $parent -Force | Out-Null
    if ((Get-Item -LiteralPath $parent -Force).Attributes -band [IO.FileAttributes]::ReparsePoint) {
        throw 'Refusing a redirected release binary destination.'
    }
    $staging = Join-Path $parent ('nd300-verified-pair-' + [guid]::NewGuid().ToString('N'))
    $zip = [IO.Compression.ZipFile]::OpenRead($Archive)
    try {
        if ($zip.Entries.Count -gt 128) { throw 'Release ZIP has too many entries.' }
        $seen = [Collections.Generic.HashSet[string]]::new([StringComparer]::OrdinalIgnoreCase)
        $binaries = @{}
        [long]$total = 0
        foreach ($entry in $zip.Entries) {
            $name = $entry.FullName
            $parts = $name.TrimEnd('/').Split('/')
            if (-not $name -or $name.Length -gt 240 -or $name -match '[\\:\x00-\x1f]' -or
                $name.StartsWith('/') -or $parts -contains '..' -or $parts -contains '.' -or
                $parts -contains '' -or -not $seen.Add($name.TrimEnd('/')) -or
                (($entry.ExternalAttributes -shr 16) -band 0xf000) -eq 0xa000 -or
                ($entry.ExternalAttributes -band 0x400)) {
                throw "Unsafe or duplicate release ZIP member: $name"
            }
            $total += $entry.Length
            if ($entry.Length -gt 128MB -or $total -gt 256MB) { throw 'Release ZIP exceeds extraction bounds.' }
            if ($name -ceq 'nd300.exe' -or $name -ceq 'speedqx.exe') { $binaries[$name] = $entry }
        }
        if ($binaries.Count -ne 2) { throw 'Release ZIP must contain exactly the intended binary pair at its root.' }
        New-Item -ItemType Directory -Path $staging | Out-Null
        foreach ($name in @('nd300.exe', 'speedqx.exe')) {
            $entry = $binaries[$name]
            $inputStream = $entry.Open()
            $outputStream = [IO.File]::Open((Join-Path $staging $name), [IO.FileMode]::CreateNew)
            try {
                $buffer = [byte[]]::new(65536)
                [long]$copied = 0
                while (($read = $inputStream.Read($buffer, 0, $buffer.Length)) -gt 0) {
                    $copied += $read
                    if ($copied -gt $entry.Length -or $copied -gt 128MB) { throw 'Release binary exceeds its declared size.' }
                    $outputStream.Write($buffer, 0, $read)
                }
                if ($copied -ne $entry.Length -or $copied -eq 0) { throw 'Release binary is empty or truncated.' }
            }
            finally { $inputStream.Dispose(); $outputStream.Dispose() }
            $versionOutput = Get-WindowsReleaseBinaryVersion (Join-Path $staging $name)
            if ($versionOutput -cne "$($name.Substring(0, $name.Length - 4)) $($Tag.Substring(1))") {
                throw "Release binary version does not match tag: $name"
            }
        }
        # The pair becomes visible together only after both have passed.
        [IO.Directory]::Move($staging, $destinationPath)
    }
    finally {
        $zip.Dispose()
        if (Test-Path -LiteralPath $staging) {
            if ([IO.Path]::GetDirectoryName([IO.Path]::GetFullPath($staging)) -ne $parent -or
                [IO.Path]::GetFileName($staging) -notlike 'nd300-verified-pair-*') {
                throw 'Verified-pair cleanup escaped its staging parent.'
            }
            Remove-Item -LiteralPath $staging -Recurse -Force
        }
    }
    return $expectedHash.ToLowerInvariant()
}

function Initialize-WindowsReleaseBinaries {
    param([string]$Repository, [string]$Tag, [string]$SourceSha, [string]$RepoRoot)
    Assert-WindowsReleaseIdentity $Repository $Tag $SourceSha
    $root = (Resolve-Path -LiteralPath $RepoRoot).Path
    $checkoutSha = git -C $root rev-parse HEAD
    if ($LASTEXITCODE -ne 0 -or "$checkoutSha".Trim() -ne $SourceSha) {
        throw 'Installer source checkout is not the exact release source.'
    }
    $toml = [IO.File]::ReadAllText((Join-Path $root 'Cargo.toml'))
    $package = [regex]::Match($toml, '(?ms)^\[package\]\s*\r?\n(?<body>.*?)(?=^\[|\z)')
    $versions = [regex]::Matches($package.Groups['body'].Value, '(?m)^version\s*=\s*"([^"]+)"\s*$')
    if ($versions.Count -ne 1 -or $versions[0].Groups[1].Value -ne $Tag.Substring(1)) {
        throw 'Installer Cargo package version does not match the release tag.'
    }
    $download = Join-Path ([IO.Path]::GetTempPath()) ('nd300-release-input-' + [guid]::NewGuid().ToString('N'))
    New-Item -ItemType Directory -Path $download | Out-Null
    try {
        gh release download $Tag --repo $Repository --dir $download `
            --pattern 'nd300-x86_64-pc-windows-msvc.zip' --pattern 'nd300-x86_64-pc-windows-msvc.zip.sha256'
        if ($LASTEXITCODE -ne 0) { throw 'Could not acquire the exact release Windows archive and checksum.' }
        $archive = Join-Path $download 'nd300-x86_64-pc-windows-msvc.zip'
        $hash = Expand-VerifiedWindowsReleasePair $archive "$archive.sha256" $Repository $Tag $SourceSha (Join-Path $root 'target/release')
        $toolingSha = git -C (Split-Path $PSScriptRoot) rev-parse HEAD
        if ($LASTEXITCODE -ne 0) { throw 'Could not record packaging helper source.' }
        [ordered]@{ repository=$Repository; tag=$Tag; source_sha=$SourceSha; installer_source_sha="$checkoutSha".Trim(); tooling_sha="$toolingSha".Trim(); archive_sha256=$hash } |
            ConvertTo-Json | Set-Content -LiteralPath (Join-Path $root 'target/windows-release-input.json')
        Write-Host "Verified exact-release binary pair for $Tag at $SourceSha (SHA-256 $hash)."
    }
    finally {
        $tempRoot = [IO.Path]::GetFullPath([IO.Path]::GetTempPath()).TrimEnd('\', '/')
        if ([IO.Path]::GetDirectoryName([IO.Path]::GetFullPath($download)) -ne $tempRoot -or
            [IO.Path]::GetFileName($download) -notlike 'nd300-release-input-*') { throw 'Download cleanup escaped the temporary root.' }
        Remove-Item -LiteralPath $download -Recurse -Force
    }
}

if ($MyInvocation.InvocationName -ne '.') {
    Initialize-WindowsReleaseBinaries $Repository $Tag $SourceSha $RepoRoot
}
