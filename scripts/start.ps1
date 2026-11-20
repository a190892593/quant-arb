# Start the ARBX backend (Axum API + scanner + paper engine + static UI).
# Usage: .\scripts\start.ps1 [-Open] [-Build]

param([switch]$Open, [switch]$Build, [switch]$BuildUi)

$ErrorActionPreference = 'Continue'
. (Join-Path $PSScriptRoot 'lib.ps1')

$root = Get-ArbxRoot
$exe = Get-ArbxExe
$url = Get-ArbxUrl

$existing = Get-ArbxListenerPid
if ($existing) {
    Write-Host "ARBX backend already running (pid $existing) at $url"
    if ($Open) { Start-Process $url }
    return
}

if ($Build -or -not (Test-Path $exe)) {
    & (Join-Path $PSScriptRoot 'build.ps1')
    if (-not (Test-Path $exe)) { throw "backend executable not found: $exe" }
}

# 前端产物（ui/）不进版本库：缺失时自动构建一次
$uiIndex = Join-Path $root 'ui\index.html'
if ($BuildUi -or -not (Test-Path $uiIndex)) {
    Write-Host 'ui/index.html missing — building frontend (first run / clean clone) ...'
    & (Join-Path $PSScriptRoot 'build-ui.ps1')
    if (-not (Test-Path $uiIndex)) { throw "frontend build did not produce ui/index.html" }
}

$dataDir = Join-Path $root 'data'
New-Item -ItemType Directory -Force $dataDir | Out-Null
$outLog = Join-Path $dataDir 'server.out.log'
$errLog = Join-Path $dataDir 'server.err.log'

Start-Process -FilePath $exe -WorkingDirectory $root `
    -RedirectStandardOutput $outLog -RedirectStandardError $errLog -WindowStyle Hidden

$ok = $false
for ($i = 0; $i -lt 40; $i++) {
    Start-Sleep -Milliseconds 250
    if (Test-ArbxBackend) { $ok = $true; break }
}

if (-not $ok) {
    Write-Host "Backend did not become healthy. Last log lines:"
    if (Test-Path $errLog) { Get-Content $errLog -Tail 20 }
    if (Test-Path $outLog) { Get-Content $outLog -Tail 20 }
    throw "start failed"
}

Write-Host "ARBX backend: $url   (API: $url/api, WS: $url/ws)"
Write-Host "UI:           $url/"
Write-Host "Logs:         $outLog"

try {
    $status = Invoke-RestMethod -Uri ($url + '/api/system/status') -TimeoutSec 10
    Write-Host ("Mode: {0}  Feed: {1}  Equity: {2}  Scanner: {3}" -f `
        $status.mode, $status.feed, $status.equity, $status.scanner.state)
} catch {
    Write-Host "Status endpoint not ready yet."
}

if ($Open) { Start-Process $url }
