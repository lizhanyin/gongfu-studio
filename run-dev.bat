@echo off
rem Gongfu Studio DEV mode: vite dev server (HMR) + dev-mode rust build + run
rem usage: run-dev.bat
rem   - frontend edits (gui/ui) hot-reload in the running app, no rebuild needed
rem   - rust edits: close the app and rerun this script
rem NOTE: keep this file ASCII-only; cmd.exe parses bat in ANSI codepage (GBK).
setlocal
cd /d "%~dp0"

rem actual cargo output name (lowercase s, normalized from package Gongfu-Studio)
set EXE_NAME=Gongfu-Studio.exe

echo [1/6] kill leftover app...
taskkill /IM %EXE_NAME% /F >nul 2>&1
rem wait up to ~5s for exit; a running exe cannot be relinked (os error 5)
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

echo [2/6] free port 1420 (a leftover vite may still hold it invisibly)...
rem note: a killed vite sometimes survives as a ghost: bound to the port but not
rem shown as LISTENING by netstat, so curl alone is not enough to detect it.
for /f "delims=" %%p in ('powershell -NoProfile -Command "Get-NetTCPConnection -LocalPort 1420 -ErrorAction SilentlyContinue | Select-Object -ExpandProperty OwningProcess | Sort-Object -Unique"') do (
    if not "%%p"=="0" taskkill /PID %%p /F >nul 2>&1
)

echo [3/6] deps...
if not exist "gui\ui\node_modules" (
    call npm --prefix gui/ui install
    if errorlevel 1 goto :err
)
rem wasm toolchain check for fmath (frontend model calc kernel, see README):
rem auto-installs the rustup wasm32 target if missing, errors on missing wasm-pack
call npm --prefix gui/ui run wasm-check
if errorlevel 1 goto :err

echo [4/6] start vite dev server (own window, close it to stop HMR)...
start "Gongfu UI - vite dev" cmd /c "npm --prefix gui/ui run dev"

echo [5/6] wait for http://localhost:1420 ...
set /a TRIES=0
:waitvite
curl -s -o nul http://localhost:1420/
if not errorlevel 1 goto viteok
set /a TRIES+=1
if %TRIES% geq 60 goto :err
timeout /t 1 /nobreak >nul
goto waitvite
:viteok

echo [6/6] build rust (dev mode: loads devUrl, no embedded dist)...
rem --no-default-features turns OFF gui's custom-protocol feature, which flips
rem tauri's cfg(dev): the window loads devUrl (vite) instead of embedded ui/dist.
call cargo build -p Gongfu-Studio --no-default-features
if errorlevel 1 goto :err

start "" "target\debug\%EXE_NAME%"
echo done. Frontend edits = HMR live; Rust edits = close app and rerun.
exit /b 0

:err
echo.
echo *** FAILED - see errors above ***
pause
exit /b 1
