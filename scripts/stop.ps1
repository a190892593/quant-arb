# Stop the ARBX backend.
# Usage: .\scripts\stop.ps1

$ErrorActionPreference = 'Continue'
. (Join-Path $PSScriptRoot 'lib.ps1')

$pid_ = Get-ArbxListenerPid
if (-not $pid_) {
    Write-Host "ARBX backend is not running."
    return
}

$proc = Get-Process -Id $pid_ -ErrorAction SilentlyContinue
$name = if ($proc) { $proc.ProcessName } else { 'unknown' }
Stop-Process -Id $pid_ -Force
for ($i = 0; $i -lt 20; $i++) {
    Start-Sleep -Milliseconds 200
    if (-not (Get-ArbxListenerPid)) { break }
}

Write-Host "Stopped pid $pid_ ($name)."
Write-Host "Persisted opportunities, executions, ledger and risk events are kept in data/arbx.db."
