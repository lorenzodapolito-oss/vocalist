@echo off
rem Scarica (o aggiorna) e compila Vocalist. Metti questo file in AmbienteRust.
title Costruisci Vocalist
set "ROOT=%~dp0"
if "%ROOT:~-1%"=="\" set "ROOT=%ROOT:~0,-1%"
set "CARGO_HOME=%ROOT%\rust\cargo"
set "RUSTUP_HOME=%ROOT%\rust\rustup"
set "PATH=%ROOT%\winlibs\mingw64\bin;%ROOT%\rust\cargo\bin;%ROOT%\git\cmd;%ROOT%\git\bin;%PATH%"
set "PROG=%ROOT%\Progetti"
if not exist "%PROG%" mkdir "%PROG%"
cd /d "%PROG%"
taskkill /f /im vocalist.exe >nul 2>&1

echo === Scarico o aggiorno Vocalist ===
if exist "%PROG%\Vocalist\.git" (
  git -c "safe.directory=*" -C "%PROG%\Vocalist" fetch origin
  if errorlevel 1 goto fail
  git -c "safe.directory=*" -C "%PROG%\Vocalist" reset --hard origin/main
) else (
  git clone https://github.com/lorenzodapolito-oss/vocalist "%PROG%\Vocalist"
)
if errorlevel 1 goto fail

echo === Compilo ===
cd /d "%PROG%\Vocalist"
cargo build --release
if errorlevel 1 goto fail

set "OUT=%PROG%\Vocalist\dist\Vocalist-portable"
if exist "%OUT%" rmdir /s /q "%OUT%"
mkdir "%OUT%"
copy /y "target\release\vocalist.exe" "%OUT%\Vocalist.exe" >nul
if errorlevel 1 goto fail

rem ffmpeg serve solo per salvare in MP3 (senza, salva in WAV)
if exist "D:\WPy64-3.13.12.0\Bin\ffmpeg.exe" copy /y "D:\WPy64-3.13.12.0\Bin\ffmpeg.exe" "%OUT%\" >nul
if exist "%ROOT%\ffmpeg\bin\ffmpeg.exe" copy /y "%ROOT%\ffmpeg\bin\ffmpeg.exe" "%OUT%\" >nul

echo.
echo FATTO: %OUT%\Vocalist.exe
if not exist "%OUT%\ffmpeg.exe" echo (ffmpeg.exe non trovato: copialo accanto a Vocalist.exe per salvare in MP3)
echo Per la voce locale Piper copia piper.exe e i file .onnx nella stessa cartella.
echo.
pause
exit /b 0

:fail
echo.
echo *** Qualcosa non ha funzionato: guarda le ultime righe sopra. ***
pause
exit /b 1
