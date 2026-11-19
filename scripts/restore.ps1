# Restore the ARBX database from a backup file.
# The backend must be stopped first: an open SQLite file must not be replaced.
# Usage: .\scripts\restore.ps1 -File .\backups\arbx-20261119-190000.db

param([Parameter(Mandatory = $true)][string]$File)

$ErrorActionPreference = 'Continue'
. (Join-Path $PSScriptRoot 'lib.ps1')

$root = Get-ArbxRoot
$db = Join-Path $root 'data\arbx.db'

if (Get-ArbxListenerPid) {
    throw "backend is running. Run .\scripts\stop.ps1 first."
}
if (-not (Test-Path $File)) { throw "backup file not found: $File" }

if (Test-Path $db) {
    $stamp = Get-Date -Format 'yyyyMMdd-HHmmss'
    $dir = Join-Path $root 'backups'
    New-Item -ItemType Directory -Force $dir | Out-Null
    $safety = Join-Path $dir ("pre-restore-$stamp.db")
    Copy-Item $db $safety -Force
    Write-Host "Current database saved as: $safety"
    Remove-Item $db -Force
    foreach ($suffix in '-wal', '-shm') {
        $extra = "$db$suffix"
        if (Test-Path $extra) { Remove-Item $extra -Force }
    }
}

Copy-Item $File $db -Force
Write-Host "Restored: $File -> $db"
Write-Host "Start the backend with .\scripts\start.ps1"
