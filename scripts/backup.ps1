# Create a consistent backup of the local ARBX database.
# Usage: .\scripts\backup.ps1 [-Out <path>]

param([string]$Out)

$ErrorActionPreference = 'Continue'
. (Join-Path $PSScriptRoot 'lib.ps1')

$root = Get-ArbxRoot
$exe = Get-ArbxExe
if (-not (Test-Path $exe)) { throw "backend executable not found: $exe (run scripts\build.ps1)" }

$stamp = Get-Date -Format 'yyyyMMdd-HHmmss'
if (-not $Out) {
    $dir = Join-Path $root 'backups'
    New-Item -ItemType Directory -Force $dir | Out-Null
    $Out = Join-Path $dir ("arbx-$stamp.db")
}

Push-Location $root
try {
    & $exe backup $Out
    if ($LASTEXITCODE -ne 0) { throw "backup failed with exit code $LASTEXITCODE" }
} finally {
    Pop-Location
}

if (Test-Path $Out) {
    $size = [math]::Round((Get-Item $Out).Length / 1KB, 1)
    Write-Host "Backup written: $Out ($size KB)"
}
