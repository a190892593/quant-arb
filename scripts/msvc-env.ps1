# Load the MSVC build environment (cl.exe / link.exe / Windows SDK include paths).
#
# Why: libsqlite3-sys with the "bundled" feature compiles sqlite3.c with cl.exe,
# which needs INCLUDE / LIB / PATH as set by vcvars64.bat. Every DSH command runs
# in a fresh process, so source this script before building C dependencies:
#
#   Set-ExecutionPolicy -Scope Process -ExecutionPolicy Bypass -Force
#   . .\scripts\msvc-env.ps1
#   cargo build
#
# If the C dependency is already compiled, this script is not strictly needed.

$ErrorActionPreference = 'Continue'

if ($env:ARBX_MSVC_READY -eq '1') {
    return
}

$vcvars = $null

$vswhere = 'C:\Program Files (x86)\Microsoft Visual Studio\Installer\vswhere.exe'
if (Test-Path $vswhere) {
    $installPath = & $vswhere -latest -products * -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath 2>$null
    if ($installPath) {
        $candidate = Join-Path $installPath 'VC\Auxiliary\Build\vcvars64.bat'
        if (Test-Path $candidate) { $vcvars = $candidate }
    }
}

if (-not $vcvars) {
    $roots = @(
        'C:\Program Files\Microsoft Visual Studio\2022',
        'C:\Program Files (x86)\Microsoft Visual Studio\2022',
        'C:\Program Files\Microsoft Visual Studio\2019',
        'C:\Program Files (x86)\Microsoft Visual Studio\2019'
    )
    foreach ($root in $roots) {
        if (Test-Path $root) {
            $hit = Get-ChildItem $root -Recurse -Filter 'vcvars64.bat' -ErrorAction SilentlyContinue |
                Select-Object -First 1 -ExpandProperty FullName
            if ($hit) { $vcvars = $hit; break }
        }
    }
}

if (-not $vcvars) {
    Write-Error 'vcvars64.bat not found. Install Visual Studio Build Tools with the C++ workload.'
    return
}

$dump = & cmd /c "`"$vcvars`" >nul 2>&1 && set"
$applied = 0
foreach ($line in $dump) {
    $eq = $line.IndexOf('=')
    if ($eq -gt 0) {
        $name = $line.Substring(0, $eq)
        $value = $line.Substring($eq + 1)
        if ($name -and $name -notmatch '^(PROMPT|_)$') {
            Set-Item -Path "env:$name" -Value $value -ErrorAction SilentlyContinue
            $applied++
        }
    }
}

if ($applied -gt 0) {
    $env:ARBX_MSVC_READY = '1'
} else {
    Write-Warning 'Failed to import the MSVC environment variables.'
}
