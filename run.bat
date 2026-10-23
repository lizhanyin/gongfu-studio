@echo off
rem Gongfu Studio: clean + build + run (Windows)
rem usage: run.bat [--full]   --full = cargo clean first
rem NOTE: keep this file ASCII-only; cmd.exe parses bat in ANSI codepage (GBK).
setlocal
cd /d "%~dp0"

rem actual cargo output name (lowercase s, normalized from package Gongfu-Studio)
set EXE_NAME=Gongfu-Studio.exe

echo [1/4] kill leftover process...
taskkill /IM %EXE_NAME% /F >nul 2>&1
rem wait up to ~5s for exit; a running exe cannot be deleted or relinked,
rem which would fail later with a confusing linker error
set /a KILLTRIES=0
:waitkill
tasklist /FI "IMAGENAME eq %EXE_NAME%" 2>nul | find /i "%EXE_NAME%" >nul 2>&1
if errorlevel 1 goto killed
set /a KILLTRIES+=1
if %KILLTRIES% geq 5 goto killfail
timeout /t 1 /nobreak >nul
goto waitkill
:killfail
echo.
echo *** ERROR: cannot terminate %EXE_NAME%, close it manually and retry ***
pause
exit /b 1
:killed

echo [2/4] remove old outputs...
if exist "target\debug\%EXE_NAME%" del /q "target\debug\%EXE_NAME%"
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
rem wasm toolchain check for fmath (frontend model calc kernel, see README):
rem auto-installs the rustup wasm32 target if missing, errors on missing wasm-pack
call npm --prefix gui/ui run wasm-check
if errorlevel 1 goto :err

echo [3/4] build frontend (gui/ui)...
call npm --prefix gui/ui run build
if errorlevel 1 goto :err

echo [3/4] build rust...
call cargo build
if errorlevel 1 goto :err

echo [4/4] start Gongfu Studio...
start "" "target\debug\%EXE_NAME%"
echo done.
exit /b 0

:err
echo.
echo *** BUILD FAILED - see errors above ***
pause
exit /b 1
