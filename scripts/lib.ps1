# ARBX - shared helpers for the local launcher scripts.
# ASCII only: Windows PowerShell 5.1 parses UTF-8 without BOM as ANSI,
# so non-ASCII comments in .ps1 files can break parsing.

function Get-ArbxRoot {
    return (Split-Path -Parent $PSScriptRoot)
}

function Get-ArbxTargetDir {
    $root = Get-ArbxRoot
    $cfg = Join-Path $root 'backend\.cargo\config.toml'
    if (Test-Path $cfg) {
        $raw = Get-Content -Raw $cfg
        $m = [regex]::Match($raw, 'target-dir\s*=\s*"([^"]+)"')
        if ($m.Success) { return $m.Groups[1].Value }
    }
    return (Join-Path $root 'backend\target')
}

function Get-ArbxExe {
    $target = Get-ArbxTargetDir
    return (Join-Path $target 'debug\arb-backend.exe')
}

function Get-ArbxPort {
    $root = Get-ArbxRoot
    $cfg = Join-Path $root 'config\default.toml'
    $port = 8791
    if (Test-Path $cfg) {
        $raw = Get-Content -Raw $cfg
        $m = [regex]::Match($raw, '(?m)^\s*port\s*=\s*(\d+)')
        if ($m.Success) { $port = [int]$m.Groups[1].Value }
    }
    if ($env:ARBX_PORT) { $port = [int]$env:ARBX_PORT }
    return $port
}

function Get-ArbxUrl {
    return ("http://127.0.0.1:" + (Get-ArbxPort))
}

function Get-ArbxListenerPid {
    $port = Get-ArbxPort
    $conn = Get-NetTCPConnection -State Listen -LocalPort $port -ErrorAction SilentlyContinue | Select-Object -First 1
    if ($conn) { return $conn.OwningProcess }
    return $null
}

function Test-ArbxBackend {
    try {
        $r = Invoke-WebRequest -Uri ((Get-ArbxUrl) + '/healthz') -UseBasicParsing -TimeoutSec 3
        return ($r.StatusCode -eq 200)
    } catch {
        return $false
    }
}
