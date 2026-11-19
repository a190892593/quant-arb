# Reset the Paper account to the initial allocation (ledger, opportunities,
# executions and risk events are cleared; the backend keeps running).
# Usage: .\scripts\reset-paper.ps1

$ErrorActionPreference = 'Continue'
. (Join-Path $PSScriptRoot 'lib.ps1')

$url = Get-ArbxUrl
if (-not (Test-ArbxBackend)) { throw "backend is not running ($url)" }

$body = '{}'
$res = Invoke-RestMethod -Uri ($url + '/api/paper/reset') -Method Post -ContentType 'application/json' -Body $body -TimeoutSec 30
Write-Host ("Paper account reset. equity={0}" -f $res.equity)
Write-Host "Scanner is stopped after reset; start it from the UI or POST /api/scanner/start."
