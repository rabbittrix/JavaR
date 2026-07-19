#Requires -Version 5.1
<#
.SYNOPSIS
  JavaR one-liner installer for Windows.

.DESCRIPTION
  Downloads the latest GitHub release (or builds from source), installs into
  %USERPROFILE%\.javar\bin, then runs `javar setup` (PATH + embedded assets).

.NOTES
  Author: Roberto de Souza <rabbittrix@hotmail.com>
  Usage:
    iwr https://javar.dev/install.ps1 | iex
    irm https://raw.githubusercontent.com/rabbittrix/JavaR/main/scripts/install.ps1 | iex
#>

$ErrorActionPreference = "Stop"

$Repo       = if ($env:JAVAR_REPO) { $env:JAVAR_REPO } else { "rabbittrix/JavaR" }
$InstallDir = Join-Path $env:USERPROFILE ".javar\bin"
$UserAgent  = "javar-install"

function Write-Banner {
  Write-Host ""
  Write-Host "  JavaR installer  " -ForegroundColor White -BackgroundColor DarkBlue
  Write-Host "  by Roberto de Souza" -ForegroundColor DarkGray
  Write-Host ""
}

function Write-Step([string]$Msg) {
  Write-Host "· $Msg" -ForegroundColor Cyan
}

function Write-Ok([string]$Msg) {
  Write-Host "✓ $Msg" -ForegroundColor DarkYellow
}

function Write-Warn([string]$Msg) {
  Write-Host "! $Msg" -ForegroundColor Yellow
}

function Get-LatestWindowsZipUrl {
  $api = "https://api.github.com/repos/$Repo/releases/latest"
  try {
    $rel = Invoke-RestMethod -Uri $api -Headers @{ "User-Agent" = $UserAgent }
  } catch {
    Write-Warn "Could not query GitHub releases for $Repo"
    return $null
  }
  $asset = $rel.assets |
    Where-Object { $_.name -match "windows-x86_64" -and $_.name -like "*.zip" } |
    Select-Object -First 1
  if (-not $asset) {
    Write-Warn "No windows-x86_64.zip asset on the latest release"
    return $null
  }
  return $asset.browser_download_url
}

function Install-FromZip([string]$ZipUrl) {
  Write-Step "Downloading $ZipUrl"
  $tmpZip = Join-Path $env:TEMP "javar-release.zip"
  $extract = Join-Path $env:TEMP "javar-extract"
  Invoke-WebRequest -Uri $ZipUrl -OutFile $tmpZip -UseBasicParsing
  if (Test-Path $extract) { Remove-Item -Recurse -Force $extract }
  Expand-Archive -Path $tmpZip -DestinationPath $extract -Force

  $bin = Get-ChildItem -Path $extract -Recurse -Filter "javar.exe" -File | Select-Object -First 1
  if (-not $bin) { throw "javar.exe not found in release archive" }
  Copy-Item -LiteralPath $bin.FullName -Destination (Join-Path $InstallDir "javar.exe") -Force

  $lib = Get-ChildItem -Path $extract -Recurse -Filter "javar_core.dll" -File | Select-Object -First 1
  if ($lib) {
    Copy-Item -LiteralPath $lib.FullName -Destination (Join-Path $InstallDir "javar_core.dll") -Force
  }

  $jar = Get-ChildItem -Path $extract -Recurse -Filter "*.jar" -File |
    Where-Object { $_.Name -match "javar-agent" -and $_.Name -notmatch "sources|javadoc|original" } |
    Select-Object -First 1
  if ($jar) {
    Copy-Item -LiteralPath $jar.FullName -Destination (Join-Path $InstallDir "javar-agent.jar") -Force
  }

  Remove-Item -Force $tmpZip -ErrorAction SilentlyContinue
  Remove-Item -Recurse -Force $extract -ErrorAction SilentlyContinue
  Write-Ok "Installed release bits to $InstallDir"
}

function Install-FromSource {
  if (-not (Get-Command git -ErrorAction SilentlyContinue)) {
    throw "git is required to build from source"
  }
  if (-not (Get-Command cargo -ErrorAction SilentlyContinue)) {
    throw "No GitHub release found and cargo is not installed. Install Rust from https://rustup.rs"
  }

  Write-Step "Building from source via cargo…"
  $src = Join-Path $env:TEMP "javar-src"
  if (Test-Path $src) { Remove-Item -Recurse -Force $src }
  git clone --depth 1 "https://github.com/$Repo.git" $src
  if ($LASTEXITCODE -ne 0) { throw "git clone failed" }

  Push-Location (Join-Path $src "javar-project")
  try {
    if (Get-Command mvn -ErrorAction SilentlyContinue) {
      Write-Step "Packaging javar-agent (Maven)"
      Push-Location "javar-agent"
      try {
        & mvn -q -DskipTests package
        if ($LASTEXITCODE -ne 0) { Write-Warn "Maven package failed — CLI may ship without embedded agent" }
      } finally {
        Pop-Location
      }
    } else {
      Write-Warn "Maven not found — agent will not be embedded in this build"
    }

    Write-Step "cargo build --release -p javar-core"
    & cargo build --release -p javar-core
    if ($LASTEXITCODE -ne 0) { throw "cargo build javar-core failed" }

    Write-Step "cargo build --release -p javar-cli"
    & cargo build --release -p javar-cli
    if ($LASTEXITCODE -ne 0) { throw "cargo build javar-cli failed" }

    Copy-Item "target\release\javar.exe" (Join-Path $InstallDir "javar.exe") -Force
    if (Test-Path "target\release\javar_core.dll") {
      Copy-Item "target\release\javar_core.dll" (Join-Path $InstallDir "javar_core.dll") -Force
    }
    $builtJar = Get-ChildItem "javar-agent\target\*.jar" -ErrorAction SilentlyContinue |
      Where-Object { $_.Name -match "javar-agent" -and $_.Name -notmatch "sources|javadoc|original" } |
      Select-Object -First 1
    if ($builtJar) {
      Copy-Item $builtJar.FullName (Join-Path $InstallDir "javar-agent.jar") -Force
    }
  } finally {
    Pop-Location
  }
  Write-Ok "Built and installed to $InstallDir"
}

# --- main ---
Write-Banner
New-Item -ItemType Directory -Force -Path $InstallDir | Out-Null

$zipUrl = Get-LatestWindowsZipUrl
if ($zipUrl) {
  Install-FromZip -ZipUrl $zipUrl
} else {
  Write-Step "Falling back to source build"
  Install-FromSource
}

$javar = Join-Path $InstallDir "javar.exe"
if (-not (Test-Path $javar)) { throw "Install failed: $javar missing" }

Write-Step "Running javar setup"
& $javar setup
if ($LASTEXITCODE -ne 0) { Write-Warn "javar setup exited with code $LASTEXITCODE" }

Write-Host ""
Write-Ok "Done. Open a new terminal and run:  javar run"
Write-Host "  Install dir: $InstallDir" -ForegroundColor DarkGray
Write-Host ""
