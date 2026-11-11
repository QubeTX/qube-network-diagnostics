Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

$repoRoot = Split-Path $PSScriptRoot
$portable = Join-Path $repoRoot '.agents/skills/validate-installers/scripts/validate-installers.ps1'
$claude = Join-Path $repoRoot '.claude/skills/validate-installers/scripts/validate-installers.ps1'
if ((Get-FileHash -LiteralPath $portable).Hash -ne (Get-FileHash -LiteralPath $claude).Hash) {
    throw 'Installer tooling mirrors differ.'
}

# Load only the production integrity helpers, never the build/install entrypoint.
$tokens = $null
$parseErrors = $null
$ast = [Management.Automation.Language.Parser]::ParseFile($portable, [ref]$tokens, [ref]$parseErrors)
if ($parseErrors.Count) { throw ($parseErrors | Out-String) }
foreach ($name in @('Assert-FileSha256', 'Expand-VerifiedWix', 'Install-VerifiedInno', 'Get-WixBinDir')) {
    $definition = $ast.Find({ param($node)
        $node -is [Management.Automation.Language.FunctionDefinitionAst] -and $node.Name -eq $name
    }, $true)
    if (-not $definition) { throw "Missing production helper $name" }
    . ([scriptblock]::Create($definition.Extent.Text))
}

function Assert-IntegrityFailure {
    param([scriptblock]$Action)
    try { & $Action }
    catch {
        if ($_.Exception.Message -like '*SHA-256 mismatch*') { return }
        throw
    }
    throw 'Expected a checksum mismatch.'
}

# Capture attempted execution. No fixture can launch a real installer.
$script:started = 0
function Start-Process {
    param($FilePath, $ArgumentList, $WindowStyle, [switch]$Wait, [switch]$PassThru)
    if ($WindowStyle -ne 'Hidden' -or -not $Wait -or -not $PassThru -or
        $ArgumentList -notcontains '/CURRENTUSER') {
        throw 'Inno invocation lost its noninteractive current-user contract.'
    }
    $script:started++
    return [pscustomobject]@{ ExitCode = 0 }
}

$script:candleCandidate = $null
function Get-Command {
    param($Name, $ErrorAction)
    if ($Name -eq 'candle.exe' -and $script:candleCandidate) {
        return [pscustomobject]@{ Source = $script:candleCandidate }
    }
    return $null
}

$tempRoot = [IO.Path]::GetFullPath([IO.Path]::GetTempPath()).TrimEnd('\', '/')
$fixtureRoot = Join-Path $tempRoot ('nd300-tool-integrity-' + [guid]::NewGuid().ToString('N'))
$previousWix = $env:WIX
try {
    $source = Join-Path $fixtureRoot 'source'
    $cacheDir = Join-Path $fixtureRoot 'cache'
    New-Item -ItemType Directory -Path $source, $cacheDir -Force | Out-Null
    Set-Content -LiteralPath (Join-Path $source 'candle.exe') -Value 'trusted candle'
    Set-Content -LiteralPath (Join-Path $source 'light.exe') -Value 'trusted light'
    $zip = Join-Path $cacheDir 'wix311-binaries.zip'
    Compress-Archive -Path (Join-Path $source '*') -DestinationPath $zip
    $WixZipSha256 = (Get-FileHash -LiteralPath $zip).Hash
    $wix = Expand-VerifiedWix -Archive $zip
    Set-Content -LiteralPath (Join-Path $wix 'candle.exe') -Value 'tampered cached executable'
    Set-Content -LiteralPath (Join-Path $wix 'untrusted.dll') -Value 'stale injected library'
    $wix = Expand-VerifiedWix -Archive $zip
    if ((Get-Content -LiteralPath (Join-Path $wix 'candle.exe')).Trim() -ne 'trusted candle' -or
        (Test-Path -LiteralPath (Join-Path $wix 'untrusted.dll'))) {
        throw 'WiX cache was trusted instead of recreated from verified bytes.'
    }
    $env:WIX = $null
    $script:candleCandidate = Join-Path $wix 'candle.exe'
    if (Get-WixBinDir) { throw 'PATH bypassed verification of the managed WiX extraction.' }
    Copy-Item -LiteralPath (Join-Path $wix 'candle.exe'), (Join-Path $wix 'light.exe') -Destination $cacheDir
    $script:candleCandidate = Join-Path $cacheDir 'candle.exe'
    if (Get-WixBinDir) { throw 'PATH bypassed verification of the old flat WiX cache.' }
    $env:WIX = $cacheDir
    $cacheBin = Join-Path $cacheDir 'bin'
    New-Item -ItemType Directory -Path $cacheBin | Out-Null
    Copy-Item -LiteralPath (Join-Path $wix 'candle.exe'), (Join-Path $wix 'light.exe') -Destination $cacheBin
    if (Get-WixBinDir) { throw 'WIX bypassed verification of the managed cache tree.' }
    $env:WIX = $fixtureRoot
    $externalBin = Join-Path $fixtureRoot 'bin'
    New-Item -ItemType Directory -Path $externalBin | Out-Null
    Copy-Item -LiteralPath (Join-Path $wix 'candle.exe'), (Join-Path $wix 'light.exe') -Destination $externalBin
    if ((Get-WixBinDir) -ne $externalBin) { throw 'Installed WiX outside the managed cache became unusable.' }
    Set-Content -LiteralPath $zip -Value 'replaced download'
    Assert-IntegrityFailure { Expand-VerifiedWix -Archive $zip }
    if (-not (Test-Path -LiteralPath (Join-Path $wix 'candle.exe'))) {
        throw 'Rejected archive modified an existing extraction.'
    }

    $inno = Join-Path $fixtureRoot 'inno.exe'
    Set-Content -LiteralPath $inno -Value 'trusted installer fixture'
    $InnoSetupSha256 = (Get-FileHash -LiteralPath $inno).Hash
    Install-VerifiedInno -Installer $inno | Out-Null
    if ($script:started -ne 1) { throw 'Verified installer was not invoked.' }
    Set-Content -LiteralPath $inno -Value 'replaced installer download'
    Assert-IntegrityFailure { Install-VerifiedInno -Installer $inno }
    if ($script:started -ne 1) { throw 'Rejected installer was executed.' }
}
finally {
    $env:WIX = $previousWix
    # This test owns only its generated direct child of the OS temporary root.
    $absoluteFixture = [IO.Path]::GetFullPath($fixtureRoot)
    if ([IO.Path]::GetDirectoryName($absoluteFixture) -ne $tempRoot -or
        [IO.Path]::GetFileName($absoluteFixture) -notlike 'nd300-tool-integrity-*') {
        throw 'Fixture cleanup escaped the temporary root.'
    }
    if (Test-Path -LiteralPath $absoluteFixture) {
        Remove-Item -LiteralPath $absoluteFixture -Recurse -Force
    }
}

Write-Host 'Installer integrity fixtures passed: corrupt downloads rejected before extraction/execution; modified cache rebuilt.'
