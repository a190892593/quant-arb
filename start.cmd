@echo off
rem Launch ARBX backend with the local PowerShell scripts.
rem No execution-policy change is required: powershell is invoked with -Bypass.
setlocal
cd /d "%~dp0"
powershell -NoProfile -ExecutionPolicy Bypass -File "%~dp0scripts\start.ps1" %*
if errorlevel 1 pause
endlocal
