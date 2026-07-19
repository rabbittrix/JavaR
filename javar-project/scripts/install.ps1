# JavaR CLI installer for Windows (PowerShell)
# Author: Roberto de Souza <rabbittrix@hotmail.com>
$ErrorActionPreference = "Stop"

$Prefix = if ($env:JAVAR_PREFIX) { $env:JAVAR_PREFIX } else { Join-Path $env:USERPROFILE ".javar" }
$BinDir = Join-Path $Prefix "bin"
New-Item -ItemType Directory -Force -Path $BinDir | Out-Null

if (-not (Get-Command cargo -ErrorAction SilentlyContinue)) {
    Write-Error "Rust/cargo is required. Install from https://rustup.rs"
}

Write-Host "==> Building JavaR CLI with cargo"
$RepoRoot = Split-Path -Parent $PSScriptRoot
if (-not (Test-Path (Join-Path $RepoRoot "Cargo.toml"))) {
    $RepoRoot = (Get-Location).Path
}

cargo install --path (Join-Path $RepoRoot "javar-cli") --root $Prefix --force

Write-Host "==> Installed javar to $BinDir"
Write-Host "Add to PATH (current user):"
Write-Host "  [Environment]::SetEnvironmentVariable('Path', `$env:Path + ';$BinDir', 'User')"
Write-Host ""
Write-Host "Commands: javar init | javar run | javar status"
