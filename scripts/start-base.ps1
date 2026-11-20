# Start the Base-chain ARBX instance (port 8792, workspace instances/base).
$ErrorActionPreference = 'Continue'
$root = Split-Path -Parent $PSScriptRoot
. (Join-Path $PSScriptRoot 'lib.ps1')
$exe = Get-ArbxExe
$ws = Join-Path $root 'instances\base'
$env:ARBX_WORKSPACE = $ws
$env:ARBX_PROFILE = 'base'
$env:ARBX_UI_DIR = Join-Path $root 'ui'
$env:ARBX_PORT = '8792'
$data = Join-Path $ws 'data'
New-Item -ItemType Directory -Force $data | Out-Null
$out = Join-Path $data 'server.out.log'; $err = Join-Path $data 'server.err.log'
Start-Process -FilePath $exe -WorkingDirectory $root -RedirectStandardOutput $out -RedirectStandardError $err -WindowStyle Hidden
Start-Sleep -Seconds 6
try {
  $s = Invoke-RestMethod -Uri 'http://127.0.0.1:8792/api/system/status' -TimeoutSec 10
  Write-Host ("Base instance: http://127.0.0.1:8792  chain={0} pools={1}" -f $s.chain, $s.pools)
  Write-Host ("Logs: {0}" -f $out)
} catch { Write-Host "Base instance not healthy; see $err" }
