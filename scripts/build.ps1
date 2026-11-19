# Build the ARBX backend (debug).
# Usage: .\scripts\build.ps1 [-Release]

param([switch]$Release)

$ErrorActionPreference = 'Continue'
$Root = Split-Path -Parent $PSScriptRoot
Set-ExecutionPolicy -Scope Process -ExecutionPolicy Bypass -Force

. (Join-Path $PSScriptRoot 'lib.ps1')
. (Join-Path $PSScriptRoot 'msvc-env.ps1')

Push-Location (Join-Path $Root 'backend')
try {
    if ($Release) { cargo build --release } else { cargo build }
    if ($LASTEXITCODE -ne 0) { throw "cargo build failed with exit code $LASTEXITCODE" }
} finally {
    Pop-Location
}

Write-Host "Build OK: $((Get-ArbxExe))"
