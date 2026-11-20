# Build the ARBX frontend into frontend/dist (served by the backend as static files).
# Usage: .\scripts\build-ui.ps1
#
# 构建产物在 frontend/dist 内部，不再是仓库根目录的 ui/：
# 产物不进版本库（.gitignore 已忽略 /frontend/dist/），克隆后执行一次本脚本即可。

$ErrorActionPreference = 'Stop'

$root = Split-Path -Parent $PSScriptRoot
$fe = Join-Path $root 'frontend'
$out = Join-Path $fe 'dist'

if (-not (Test-Path $fe)) { throw "frontend directory not found: $fe" }

Push-Location $fe
try {
    if (-not (Test-Path (Join-Path $fe 'node_modules'))) {
        Write-Host 'frontend/node_modules missing, running npm install ...'
        npm install
        if ($LASTEXITCODE -ne 0) { throw "npm install failed with exit code $LASTEXITCODE" }
    }

    # esbuild 在部分 Windows 环境下会因临时目录权限失败，这里兜底到仓库内目录
    if (-not $env:TEMP -or $env:TEMP -like '*\Temp') {
        $env:TEMP = Join-Path $root '.build\tmp'
        $env:TMP = $env:TEMP
        New-Item -ItemType Directory -Force $env:TEMP | Out-Null
    }

    npm run publish
    if ($LASTEXITCODE -ne 0) { throw "npm run publish failed with exit code $LASTEXITCODE" }
} finally {
    Pop-Location
}

$index = Join-Path $out 'index.html'
if (-not (Test-Path $index)) { throw "build finished but frontend/dist/index.html is missing" }
Write-Host "UI built: $index"
