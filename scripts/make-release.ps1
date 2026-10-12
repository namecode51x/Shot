#Requires -Version 5.1
<#
.SYNOPSIS
  Builds the NameShot release deliverables: portable ZIP + Setup.exe installer.

.DESCRIPTION
  1. Reads the version from Cargo.toml.
  2. Runs `cargo build --release` (needs stable Rust on PATH).
  3. Stages target\release\nameshot.exe + README.md + docs\ into dist\stage\.
  4. Writes a timestamped portable zip plus the plain
     dist\NameShot-<ver>-win-x64.zip alias (same layout as previous releases).
  5. If the Inno Setup 6 compiler (ISCC.exe) is found, compiles
     installer\NameShot.iss into dist\NameShot-<ver>-win-x64-Setup.exe;
     otherwise it prints where to get Inno Setup and stops after the zip.
  6. Prints SHA-256 hashes of every artifact it produced.

.EXAMPLE
  powershell -ExecutionPolicy Bypass -File scripts\make-release.ps1
#>
[CmdletBinding()]
param(
  [switch]$SkipBuild,
  [switch]$SkipInstaller
)

$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot

function Get-AppVersion {
  $line = Select-String -Path (Join-Path $root 'Cargo.toml') -Pattern '^version\s*=\s*"([^"]+)"' |
    Select-Object -First 1
  if (-not $line) { throw 'Could not read version from Cargo.toml' }
  return $line.Matches[0].Groups[1].Value
}

function Find-Iscc {
  $cmd = Get-Command iscc.exe -ErrorAction SilentlyContinue
  if ($cmd) { return $cmd.Source }
  foreach ($p in @("${env:ProgramFiles(x86)}\Inno Setup 6\ISCC.exe",
                   "$env:ProgramFiles\Inno Setup 6\ISCC.exe")) {
    if (Test-Path $p) { return $p }
  }
  return $null
}

$version = Get-AppVersion
Write-Host "NameShot release, version $version"

if (-not $SkipBuild) {
  if (-not (Get-Command cargo -ErrorAction SilentlyContinue)) {
    throw 'cargo not found on PATH. Install stable Rust from https://rustup.rs/ and re-run.'
  }
  Write-Host '-- cargo build --release'
  Push-Location $root
  try {
    & cargo build --release
    if ($LASTEXITCODE -ne 0) { throw "cargo build --release failed (exit $LASTEXITCODE)" }
  } finally {
    Pop-Location
  }
}

$exe = Join-Path $root 'target\release\nameshot.exe'
if (-not (Test-Path $exe)) { throw "Release exe not found: $exe (run without -SkipBuild first)" }

$dist = Join-Path $root 'dist'
New-Item -ItemType Directory -Force -Path $dist | Out-Null

# --- Stage the payload (same content for the zip and the installer) ---
$stage = Join-Path $dist 'stage'
if (Test-Path $stage) { Remove-Item -Recurse -Force $stage }
New-Item -ItemType Directory -Force -Path $stage | Out-Null
New-Item -ItemType Directory -Force -Path (Join-Path $stage 'docs') | Out-Null
Copy-Item $exe (Join-Path $stage 'nameshot.exe')
Copy-Item (Join-Path $root 'README.md') $stage
Copy-Item (Join-Path $root 'docs\*') (Join-Path $stage 'docs') -Recurse

# --- Portable zip (timestamped + plain alias, matching previous releases) ---
$stamp = Get-Date -Format 'yyyyMMdd_HHmmss'
$zipName = "NameShot-$version-win-x64-$stamp.zip"
$zipPath = Join-Path $dist $zipName
if (Test-Path $zipPath) { Remove-Item -Force $zipPath }
Compress-Archive -Path (Join-Path $stage '*') -DestinationPath $zipPath -CompressionLevel Optimal
$aliasZip = Join-Path $dist "NameShot-$version-win-x64.zip"
Copy-Item -Force $zipPath $aliasZip
Write-Host "-- portable: $zipName"

# --- Installer (Inno Setup 6) ---
$setupName = $null
if (-not $SkipInstaller) {
  $iscc = Find-Iscc
  if (-not $iscc) {
    Write-Warning ('ISCC.exe not found: Setup.exe skipped. Install Inno Setup 6 from ' +
      'https://jrsoftware.org/isdl.php (default options are enough), then re-run this script.')
  } else {
    Write-Host "-- iscc installer\NameShot.iss"
    & $iscc "/DMyAppVersion=$version" "/O$dist" (Join-Path $root 'installer\NameShot.iss')
    if ($LASTEXITCODE -ne 0) { throw "ISCC failed (exit $LASTEXITCODE)" }
    $setupName = "NameShot-$version-win-x64-Setup.exe"
  }
}

# --- Hashes ---
Write-Host ''
Write-Host 'Artifacts (SHA-256):'
$files = @($zipPath, $aliasZip)
if ($setupName) { $files += Join-Path $dist $setupName }
foreach ($f in $files) {
  $h = (Get-FileHash -Algorithm SHA256 $f).Hash
  Write-Host ("  {0}  {1}" -f $h, (Split-Path -Leaf $f))
}
Write-Host ''
Write-Host 'Done.'
