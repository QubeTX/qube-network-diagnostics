<#
.SYNOPSIS
  Add running-image retirement to cargo-dist's generated Windows installer.

.DESCRIPTION
  cargo-dist intentionally emits a generic Copy-Item installer. On Windows a
  running nd300.exe cannot be overwritten, so an older nd300 update process
  would lock the very file the generated installer needs to replace. This
  deterministic post-processor renames the two package binaries to versioned,
  narrowly allowlisted siblings before copying, restores them on copy failure,
  and asks the newly installed binary to remove the retired images afterward.

  The two anchors are pinned to cargo-dist 0.31.0 output. Any generator drift is
  a hard release failure instead of silently publishing an unhardened script.
#>
[CmdletBinding()]
param(
    [Parameter(Mandatory)]
    [string]$Path,

    [Parameter(Mandatory)]
    [string]$ExpectedVersion
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

function Replace-ExactlyOnce {
    param(
        [Parameter(Mandatory)][string]$Content,
        [Parameter(Mandatory)][string]$Needle,
        [Parameter(Mandatory)][string]$Replacement,
        [Parameter(Mandatory)][string]$Label
    )
    # PowerShell preserves a script's source newlines inside here-strings. A
    # Windows checkout may therefore make these anchors CRLF even though the
    # generated cargo-dist content is normalized to LF above. Normalize both
    # operands so the fail-closed anchor check is checkout-policy independent.
    $normalizedNeedle = $Needle.Replace("`r`n", "`n")
    $normalizedReplacement = $Replacement.Replace("`r`n", "`n")
    $count = [regex]::Matches($Content, [regex]::Escape($normalizedNeedle)).Count
    if ($count -ne 1) {
        throw "Expected exactly one $Label anchor in cargo-dist installer, found $count"
    }
    return $Content.Replace($normalizedNeedle, $normalizedReplacement)
}

$resolved = (Resolve-Path -LiteralPath $Path).Path
$crlf = [string][char]13 + [char]10
$lf = [string][char]10
$content = [System.IO.File]::ReadAllText($resolved).Replace($crlf, $lf)
$versionPattern = '^[0-9]+\.[0-9]+\.[0-9]+(?:-[0-9A-Za-z.-]+)?(?:\+[0-9A-Za-z.-]+)?$'
if ($ExpectedVersion -notmatch $versionPattern) {
    throw "Refusing unsafe installer version: $ExpectedVersion"
}
$escapedVersion = [regex]::Escape($ExpectedVersion)
if ($content -notmatch "(?m)^\`$app_version = '$escapedVersion'$") {
    throw "Installer does not declare expected version $ExpectedVersion"
}
if ($content.Contains('ND300 running-image compatibility shim')) {
    throw 'Installer already contains the ND300 running-image compatibility shim'
}

$versionNeedle = "`$app_version = '$ExpectedVersion'"
$channelHelpers = @'

# ND300 installation-channel compatibility helpers. Public commands and asset
# names stay versionless; once this latest installer has been fetched, its
# embedded app_version pins every delegated byte to one immutable release tag.
if (-not (Get-Variable -Name ND300InstallerFixtureMode -Scope Script -ErrorAction SilentlyContinue)) {
  $script:ND300InstallerFixtureMode = $false
}
function Get-Nd300ParentProcess {
  try {
    $self = Get-CimInstance Win32_Process -Filter "ProcessId = $PID" -ErrorAction Stop
    if (-not $self.ParentProcessId) { throw 'the installer host has no inspectable parent process' }
    $parent = Get-CimInstance Win32_Process -Filter "ProcessId = $($self.ParentProcessId)" -ErrorAction Stop
    if (-not $parent.ExecutablePath) { throw 'the parent executable path is unavailable' }
    return [pscustomobject]@{
      Name = [System.IO.Path]::GetFileNameWithoutExtension([string]$parent.Name)
      Path = [string]$parent.ExecutablePath
    }
  } catch {
    throw "ND-300 cannot prove whether this is an update or a fresh install: $($_.Exception.Message)"
  }
}

function Get-Nd300NormalizedPath($Path) {
  if ([string]::IsNullOrWhiteSpace([string]$Path)) { return $null }
  try {
    $expanded = [Environment]::ExpandEnvironmentVariables(([string]$Path).Trim().Trim('"'))
    return [System.IO.Path]::GetFullPath($expanded).TrimEnd('\')
  } catch {
    return $null
  }
}

function Test-Nd300InstallLocationOwnsParent($InstallLocation, $ParentPath) {
  $parent = Get-Nd300NormalizedPath $ParentPath
  $location = Get-Nd300NormalizedPath $InstallLocation
  if (-not $parent -or -not $location) { return $false }
  if ([System.IO.Path]::GetFileName($parent) -notin @('nd300.exe', 'speedqx.exe')) { return $false }
  foreach ($candidate in @(
      (Join-Path $location ([System.IO.Path]::GetFileName($parent))),
      (Join-Path (Join-Path $location 'bin') ([System.IO.Path]::GetFileName($parent)))
  )) {
    $normalized = Get-Nd300NormalizedPath $candidate
    if ($normalized -and $normalized.Equals($parent, [System.StringComparison]::OrdinalIgnoreCase)) {
      return $true
    }
  }
  return $false
}

function Get-Nd300RecordValue($Record, $Name) {
  $property = $Record.PSObject.Properties[[string]$Name]
  if ($property) { return $property.Value }
  return $null
}

function Get-Nd300SystemExecutable($RelativePath) {
  $systemDirectory = [Environment]::SystemDirectory
  if ([string]::IsNullOrWhiteSpace($systemDirectory) -or
      -not [System.IO.Path]::IsPathRooted($systemDirectory)) {
    throw 'The trusted Windows system directory could not be resolved'
  }
  $candidate = [System.IO.Path]::GetFullPath((Join-Path $systemDirectory $RelativePath))
  if (-not (Test-Path -LiteralPath $candidate -PathType Leaf)) {
    throw "The trusted Windows system executable is missing: $candidate"
  }
  return $candidate
}

function Get-Nd300RegisteredChannels($ParentPath) {
  $channels = @()
  $roots = @(
    'Registry::HKEY_CURRENT_USER\Software\Microsoft\Windows\CurrentVersion\Uninstall',
    'Registry::HKEY_LOCAL_MACHINE\Software\Microsoft\Windows\CurrentVersion\Uninstall',
    'Registry::HKEY_LOCAL_MACHINE\Software\WOW6432Node\Microsoft\Windows\CurrentVersion\Uninstall'
  )
  foreach ($root in $roots) {
    try {
      if (-not (Test-Path -LiteralPath $root -ErrorAction Stop)) { continue }
      $keys = @(Get-ChildItem -LiteralPath $root -ErrorAction Stop)
    } catch {
      throw "ND-300 could not inspect registered install ownership at $root`: $($_.Exception.Message)"
    }
    foreach ($key in $keys) {
      try {
        $record = Get-ItemProperty -LiteralPath $key.PSPath -ErrorAction Stop
      } catch {
        throw "ND-300 could not inspect registered owner $($key.PSPath): $($_.Exception.Message)"
      }
      $installLocation = Get-Nd300RecordValue $record 'InstallLocation'
      if (-not (Test-Nd300InstallLocationOwnsParent $installLocation $ParentPath)) { continue }
      $windowsInstaller = Get-Nd300RecordValue $record 'WindowsInstaller'
      $displayName = Get-Nd300RecordValue $record 'DisplayName'
      $keyName = $key.PSChildName.ToUpperInvariant()
      if ($keyName -eq '{13F102E1-E08D-4C4E-ABA6-7D77604DFECD}_IS1') {
        $channels += 'exe-global'
      } elseif ($keyName -eq '{B6A0E3BD-BDD8-44A3-B524-C226B2A116A9}_IS1') {
        $channels += 'exe-corporate'
      } elseif ($windowsInstaller -eq 1 -and
                $displayName -eq 'nd300 (Corporate Edition)') {
        $channels += 'msi-corporate'
      } elseif ($windowsInstaller -eq 1 -and
                $root -like 'Registry::HKEY_LOCAL_MACHINE*' -and
                $displayName -in @('nd300', 'nd-300')) {
        $channels += 'msi-global'
      }
    }
  }
  return @($channels | Sort-Object -Unique)
}

function Select-Nd300UpdateChannel($ParentName, $Channels, $Marker) {
  if ($ParentName -notin @('nd300', 'speedqx')) { return $null }
  $unique = @($Channels | Sort-Object -Unique)
  if ($unique.Count -eq 0) { return $null }
  if ($unique.Count -eq 1) { return $unique[0] }
  if ($Marker -and $Marker -in $unique) { return $Marker }
  throw "Multiple registered ND-300 channels exist and the newest one cannot be proven: $($unique -join ', ')"
}

function Test-Nd300CargoRegistration($DestDir) {
  $bin = Get-Nd300NormalizedPath $DestDir
  if (-not $bin -or [System.IO.Path]::GetFileName($bin) -ne 'bin') { return $false }
  $cargoHome = Split-Path -Parent $bin
  $jsonPath = Join-Path $cargoHome '.crates2.json'
  if (Test-Path -LiteralPath $jsonPath -PathType Leaf) {
    try {
      $registry = Get-Content -LiteralPath $jsonPath -Raw -ErrorAction Stop | ConvertFrom-Json -ErrorAction Stop
      $installs = $registry.PSObject.Properties['installs']
      if ($installs) {
        foreach ($key in $installs.Value.PSObject.Properties.Name) {
          if ($key -match '^nd300 [0-9]') { return $true }
        }
      }
    } catch {
      throw "ND-300 could not verify Cargo ownership from $jsonPath`: $($_.Exception.Message)"
    }
  }
  $tomlPath = Join-Path $cargoHome '.crates.toml'
  if (Test-Path -LiteralPath $tomlPath -PathType Leaf) {
    try {
      $toml = Get-Content -LiteralPath $tomlPath -Raw -ErrorAction Stop
      if ($toml -match '(?m)^\s*["'']nd300 [0-9]') { return $true }
    } catch {
      throw "ND-300 could not verify Cargo ownership from $tomlPath`: $($_.Exception.Message)"
    }
  }
  return $false
}

function Select-Nd300InvocationMode($ParentName, $ParentPath, $DestDir, $Channels, $Marker, $CargoRegistered) {
  if ($ParentName -notin @('nd300', 'speedqx')) { return 'fresh' }
  $channel = Select-Nd300UpdateChannel $ParentName $Channels $Marker
  if ($channel) { return "registered:$channel" }
  if (-not (Test-Nd300InstallLocationOwnsParent $DestDir $ParentPath)) {
    throw 'An ND-300 parent launched the installer, but neither registered ownership nor the exact destination owns that parent; no files were changed'
  }
  if ($CargoRegistered) { return 'cargo-update' }
  return 'standalone-update'
}

function Invoke-Nd300RegisteredChannelUpdate($DestDir) {
  if ($script:ND300InstallerFixtureMode) { return 'fixture' }
  $parent = Get-Nd300ParentProcess
  $channels = if ($parent.Name -in @('nd300', 'speedqx')) {
    @(Get-Nd300RegisteredChannels $parent.Path)
  } else {
    @()
  }
  $marker = $null
  try {
    $marker = Get-ItemPropertyValue -LiteralPath 'Registry::HKEY_CURRENT_USER\Software\ND300' -Name InstallSource -ErrorAction Stop
  } catch {}
  $cargoRegistered = Test-Nd300CargoRegistration $DestDir
  $mode = Select-Nd300InvocationMode $parent.Name $parent.Path $DestDir $channels $marker $cargoRegistered
  if (-not $mode.StartsWith('registered:')) { return $mode }
  $channel = $mode.Substring('registered:'.Length)

  $asset = switch ($channel) {
    'msi-global' { 'nd300-x86_64-pc-windows-msvc.msi' }
    'msi-corporate' { 'nd300-x86_64-pc-windows-msvc-corporate.msi' }
    'exe-global' { 'nd300-x86_64-pc-windows-msvc-setup.exe' }
    'exe-corporate' { 'nd300-x86_64-pc-windows-msvc-corporate-setup.exe' }
    default { throw "Unsupported registered ND-300 channel: $channel" }
  }
  $base = "https://github.com/QubeTX/qube-network-diagnostics/releases/download/v$app_version"
  $url = "$base/$asset"
  $installer = Join-Path ([System.IO.Path]::GetTempPath()) "nd300-channel-$app_version-$asset"
  $sidecar = "$installer.sha256"
  try {
    Invoke-WebRequest -Uri $url -OutFile $installer -UseBasicParsing -ErrorAction Stop
    Invoke-WebRequest -Uri "$url.sha256" -OutFile $sidecar -UseBasicParsing -ErrorAction Stop
    $expected = ((Get-Content -LiteralPath $sidecar -Raw).Trim() -split '\s+')[0]
    if ($expected -notmatch '^[0-9A-Fa-f]{64}$') {
      throw 'The matching installer checksum sidecar is malformed'
    }
    $actual = (Get-FileHash -LiteralPath $installer -Algorithm SHA256).Hash
    if ($actual -ne $expected) {
      throw "Matching installer checksum mismatch (expected $expected, got $actual)"
    }

    if ($channel -like 'msi-*') {
      $process = Start-Process -FilePath (Get-Nd300SystemExecutable 'msiexec.exe') `
        -ArgumentList @('/i', "`"$installer`"", '/passive', '/norestart') -Wait -PassThru
    } else {
      $process = Start-Process -FilePath $installer `
        -ArgumentList @('/SILENT', '/SUPPRESSMSGBOXES', '/NORESTART', '/SP-') -Wait -PassThru
    }
    if ($process.ExitCode -ne 0) {
      throw "Matching $channel installer exited $($process.ExitCode)"
    }
    return 'delegated'
  } finally {
    Remove-Item -LiteralPath $installer -Force -ErrorAction SilentlyContinue
    Remove-Item -LiteralPath $sidecar -Force -ErrorAction SilentlyContinue
  }
}
'@
$content = Replace-ExactlyOnce $content $versionNeedle ($versionNeedle + $channelHelpers) 'app-version'

$startNeedle = @'
  Write-Information "installing to $dest_dir"
  # Just copy the binaries from the temp location to the install dir
  foreach ($bin_path in $artifacts["bin_paths"]) {
'@
$startReplacement = @'
  Write-Information "installing to $dest_dir"

  # An updater from the pre-channel-marker era still fetches this versionless
  # script. If its parent and a unique registered owner prove an MSI/EXE channel,
  # delegate to that same current installer instead of crossing to standalone.
  $nd300_invocation_mode = Invoke-Nd300RegisteredChannelUpdate "$dest_dir"
  if ($nd300_invocation_mode -eq 'delegated') {
    return
  }

  # ND300 running-image compatibility shim. Rename only the two package
  # binaries, in their already-resolved install directory, so a running older
  # updater does not lock the canonical destination names.
  $retired_update_files = @()
  $installed_update_targets = @()
  try {
  foreach ($installed_name in @("nd300.exe", "speedqx.exe")) {
    $target = Join-Path "$dest_dir" $installed_name
    if (Test-Path -LiteralPath $target -PathType Leaf) {
      $stem = [System.IO.Path]::GetFileNameWithoutExtension($installed_name)
      $retired = Join-Path "$dest_dir" "$stem.update-old-$app_version.exe"
      if (Test-Path -LiteralPath $retired -PathType Leaf) {
        Remove-Item -LiteralPath $retired -Force -ErrorAction Stop
      }
      Move-Item -LiteralPath $target -Destination $retired -ErrorAction Stop
      $retired_update_files += [pscustomobject]@{
        Target = $target
        Retired = $retired
      }
    }
  }

  if ($nd300_invocation_mode -eq 'cargo-update') {
    $cargo = Join-Path "$dest_dir" 'cargo.exe'
    if (-not (Test-Path -LiteralPath $cargo -PathType Leaf)) {
      throw "Cargo owns this installation, but its executable is missing at $cargo"
    }
    & $cargo install nd300 --version "=$app_version" --force --locked
    if ($LASTEXITCODE -ne 0) {
      throw "Cargo failed to install the exact nd300 $app_version crate (exit $LASTEXITCODE)"
    }
    $artifacts["bin_paths"] = @()
  }

  # Just copy the binaries from the temp location to the install dir
  foreach ($bin_path in $artifacts["bin_paths"]) {
'@

$endNeedle = @'
  }
  foreach ($lib_path in $artifacts["lib_paths"]) {
'@
$endReplacement = @'
  }

  if ($nd300_invocation_mode -eq 'fresh') {
    $takeover_binary = Join-Path "$dest_dir" "nd300.exe"
    & $takeover_binary install-takeover --target standalone --scope all
    if ($LASTEXITCODE -ne 0) {
      throw "The standalone files were staged, but the previous registered ND-300 channel could not be removed. See https://reports.qubetx.com/nd300#install"
    }
  }
  } catch {
    $install_error = $_
    $rollback_errors = @()
    for ($i = $installed_update_targets.Count - 1; $i -ge 0; $i--) {
      try {
        if (Test-Path -LiteralPath $installed_update_targets[$i] -PathType Leaf) {
          Remove-Item -LiteralPath $installed_update_targets[$i] -Force -ErrorAction Stop
        }
      } catch {
        $rollback_errors += "Could not remove staged $($installed_update_targets[$i]): $($_.Exception.Message)"
      }
    }
    for ($i = $retired_update_files.Count - 1; $i -ge 0; $i--) {
      $mapping = $retired_update_files[$i]
      try {
        if (Test-Path -LiteralPath $mapping.Target -PathType Leaf) {
          Remove-Item -LiteralPath $mapping.Target -Force -ErrorAction Stop
        }
        if (Test-Path -LiteralPath $mapping.Retired -PathType Leaf) {
          Move-Item -LiteralPath $mapping.Retired -Destination $mapping.Target -ErrorAction Stop
        }
      } catch {
        $rollback_errors += "Could not restore $($mapping.Target): $($_.Exception.Message)"
      }
    }
    if ($rollback_errors.Count -gt 0) {
      throw "$($install_error.Exception.Message); rollback also failed: $($rollback_errors -join '; ')"
    }
    throw $install_error
  }

  if ($retired_update_files.Count -gt 0) {
    $cleanup_binary = Join-Path "$dest_dir" "nd300.exe"
    if (Test-Path -LiteralPath $cleanup_binary -PathType Leaf) {
      try {
        $cleanup_process = Start-Process `
          -FilePath $cleanup_binary `
          -ArgumentList @('migrate-cleanup', '--quiet', '--retired-update') `
          -NoNewWindow `
          -Wait `
          -PassThru `
          -ErrorAction Stop
        if ($cleanup_process.ExitCode -ne 0) {
          Write-Warning "The update completed, but retired-image cleanup exited $($cleanup_process.ExitCode)"
        }
      } catch {
        Write-Warning "The update completed, but retired-image cleanup could not run: $_"
      }
    }
  }
  foreach ($lib_path in $artifacts["lib_paths"]) {
'@

$copyNeedle = '    Copy-Item "$bin_path" -Destination "$dest_dir" -ErrorAction Stop'
$copyReplacement = @'
    Copy-Item "$bin_path" -Destination "$dest_dir" -ErrorAction Stop
    $installed_update_targets += (Join-Path "$dest_dir" $installed_file)
'@

$content = Replace-ExactlyOnce $content $startNeedle $startReplacement 'binary-copy start'
$content = Replace-ExactlyOnce $content $endNeedle $endReplacement 'binary-copy end'
$content = Replace-ExactlyOnce $content $copyNeedle $copyReplacement 'binary-copy tracking'
[System.IO.File]::WriteAllText(
    $resolved,
    $content,
    [System.Text.UTF8Encoding]::new($false)
)
Write-Host "Hardened cargo-dist Windows installer for nd300 $ExpectedVersion"
