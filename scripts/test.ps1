# Build the ARBX backend and run the full test suite.
# Usage: .\scripts\test.ps1 [-Filter <pattern>]

param([string]$Filter = '')

$ErrorActionPreference = 'Continue'
$Root = Split-Path -Parent $PSScriptRoot
Set-ExecutionPolicy -Scope Process -ExecutionPolicy Bypass -Force

. (Join-Path $PSScriptRoot 'msvc-env.ps1')

# 运行中的后端会锁住 debug/arb-backend.exe：cargo test 重建 bin target 时会报
# "failed to remove file ...\arb-backend.exe"。测试前先停服务；测试后不自动重启，
# 避免把 Paper 状态变化混进测试结论。
$stop = Join-Path $PSScriptRoot 'stop.ps1'
if (Test-Path $stop) { & $stop | Out-Host }

Push-Location (Join-Path $Root 'backend')
try {
    if ($Filter) { cargo test $Filter } else { cargo test }
    $code = $LASTEXITCODE
} finally {
    Pop-Location
}

if ($code -ne 0) { throw "tests failed with exit code $code" }
Write-Host "All tests passed."
