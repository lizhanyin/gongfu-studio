@echo off
rem Gongfu Studio: clean + build + run (Windows)
rem usage: run.bat [--full]   --full = cargo clean first
rem NOTE: keep this file ASCII-only; cmd.exe parses bat in ANSI codepage (GBK).
setlocal
cd /d "%~dp0"

echo [1/4] kill leftover process...
taskkill /IM Gongfu-studio.exe /F >nul 2>&1

echo [2/4] remove old outputs...
if exist "target\release\Gongfu-studio.exe" del /q "target\release\Gongfu-studio.exe"
if exist "gui\ui\dist" rmdir /s /q "gui\ui\dist"

if "%~1"=="--full" (
    echo [2/4] --full: cargo clean...
    call cargo clean
)

echo [3/4] deps...
if not exist "gui\ui\node_modules" (
    call npm --prefix gui/ui install
    if errorlevel 1 goto :err
)

echo [3/4] build frontend (gui/ui)...
call npm --prefix gui/ui run build
if errorlevel 1 goto :err

echo [3/4] build rust...
call cargo build --release
if errorlevel 1 goto :err

echo [4/4] start Gongfu Studio...
start "" "target\release\Gongfu-studio.exe"
echo done.
exit /b 0

:err
echo.
echo *** BUILD FAILED - see errors above ***
pause
exit /b 1
