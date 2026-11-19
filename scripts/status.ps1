# Show ARBX backend status (system, scanner, risk, performance).
# Usage: .\scripts\status.ps1

$ErrorActionPreference = 'Continue'
. (Join-Path $PSScriptRoot 'lib.ps1')

$url = Get-ArbxUrl
if (-not (Test-ArbxBackend)) {
    Write-Host "ARBX backend is DOWN (expected $url)."
    exit 1
}

$status = Invoke-RestMethod -Uri ($url + '/api/system/status') -TimeoutSec 10
$scan = Invoke-RestMethod -Uri ($url + '/api/scanner/status') -TimeoutSec 10
$perf = Invoke-RestMethod -Uri ($url + '/api/paper/performance') -TimeoutSec 10
$risk = Invoke-RestMethod -Uri ($url + '/api/risk/status') -TimeoutSec 10
$opps = Invoke-RestMethod -Uri ($url + '/api/opportunities?limit=300') -TimeoutSec 10

Write-Host "== ARBX =="
Write-Host ("mode={0} feed={1} version={2}" -f $status.mode, $status.feed, $status.version)
Write-Host ("db={0}" -f $status.db)
Write-Host ("scanner={0} interval={1}ms last_scan={2}ms detected={3} actionable={4} rejected={5}" -f `
    $scan.state, $scan.scan_interval_ms, $scan.last_scan_ms, $scan.detected, $scan.actionable, $scan.rejected)
Write-Host ("equity={0} start={1} realized={2} fees={3} gas={4} net={5} trades={6} win_rate={7}%" -f `
    $perf.equity, $perf.start_equity, $perf.realized_pnl, $perf.total_fees, $perf.total_gas, $perf.net_pnl, $perf.trades, $perf.win_rate)
Write-Host ("risk_param_version={0} global_paused={1} events={2}" -f `
    $risk.param_version, $risk.controls.global.paused, $risk.events.Count)

$groups = $opps.opportunities | Group-Object strategy_type
foreach ($g in $groups) { Write-Host ("  {0}: {1}" -f $g.Name, $g.Count) }
