# Build the ARBX backend and run the full test suite.
# Usage: .\scripts\test.ps1 [-Filter <pattern>]

param([string]$Filter = '')

$ErrorActionPreference = 'Continue'
$Root = Split-Path -Parent $PSScriptRoot
Set-ExecutionPolicy -Scope Process -ExecutionPolicy Bypass -Force

. (Join-Path $PSScriptRoot 'msvc-env.ps1')

Push-Location (Join-Path $Root 'backend')
try {
    if ($Filter) { cargo test $Filter } else { cargo test }
    $code = $LASTEXITCODE
} finally {
    Pop-Location
}

if ($code -ne 0) { throw "tests failed with exit code $code" }
Write-Host "All tests passed."
