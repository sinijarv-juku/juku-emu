@echo off
rem Builds the Windows front-end (spec section 11.1) into target\windows:
rem Juku.exe and JukuThumbnail.dll, Explorer's thumbnail handler for snapshots.
rem
rem   build.cmd          release build tuned for this computer's CPU
rem   build.cmd debug    unoptimized build with debug info; a console program,
rem                      so its log shows in the terminal
rem   build.cmd dist     release build for other computers,
rem                      target\Juku-<version>-windows-x64.zip and, if Inno
rem                      Setup 6 is installed, the installer
rem                      target\Juku-<version>-windows-x64-setup.exe
rem
rem Needs nightly Rust with rust-src, the Visual Studio Build Tools (MSVC
rem linker) and the Windows SDK. The flags are here rather than in
rem .cargo\config.toml, which would give them to `cargo test` too; that file
rem puts cargo's output in target\cargo. The file has no labels, so it works
rem with LF line endings too.

setlocal
cd /d "%~dp0"

set TARGET=x86_64-pc-windows-msvc
set OUT=target\windows
set CARGO_PROFILE=--release
set CARGO_OUT=release
rem The C runtime is linked in, except the Universal CRT, which is part of
rem Windows 10 and later: it is taken from ucrtbase.dll (spec section 11.1).
set CRT=-Ctarget-feature=+crt-static -Clink-arg=/NODEFAULTLIB:libucrt.lib -Clink-arg=ucrt.lib
set RUSTFLAGS=%CRT% -Zlocation-detail=none -Zfmt-debug=none

if "%~1"=="debug" (
    set CARGO_PROFILE=
    set CARGO_OUT=debug
    set RUSTFLAGS=%CRT% -Zlocation-detail=none
) else if "%~1"=="" (
    set "RUSTFLAGS=%RUSTFLAGS% -Ctarget-cpu=native"
) else if not "%~1"=="dist" (
    echo usage: build.cmd [debug^|dist] 1>&2
    exit /b 2
)

rem Line tables for Juku.pdb and JukuThumbnail.pdb, for reading crash dumps;
rem the exe and the DLL are the same without them. The Mac's library has none.
set CARGO_PROFILE_RELEASE_DEBUG=line-tables-only

rem Two runs, so that the DLL is built with its own features of windows only.
cargo build -p juku-windows --target %TARGET% -Z build-std=std %CARGO_PROFILE% || exit /b 1
cargo build -p juku-thumbnail --target %TARGET% -Z build-std=std %CARGO_PROFILE% || exit /b 1

set BUILT=target\cargo\%TARGET%\%CARGO_OUT%
if not exist %OUT% mkdir %OUT%
copy /y %BUILT%\Juku.exe %OUT%\Juku.exe >nul || exit /b 1
rem Explorer keeps the DLL loaded for a while after it has drawn thumbnails.
copy /y %BUILT%\JukuThumbnail.dll %OUT%\JukuThumbnail.dll >nul || (echo JukuThumbnail.dll is in use by Explorer's thumbnail process; try again in a minute. 1>&2 & exit /b 1)

rem Symbols for reading crash dumps; the exe and the DLL have none.
if exist %BUILT%\Juku.pdb copy /y %BUILT%\Juku.pdb %OUT%\Juku.pdb >nul
if exist %BUILT%\JukuThumbnail.pdb copy /y %BUILT%\JukuThumbnail.pdb %OUT%\JukuThumbnail.pdb >nul

echo Built %OUT%\Juku.exe and %OUT%\JukuThumbnail.dll
if not "%~1"=="dist" exit /b 0

rem The version is VERSION's, as the Mac app's (spec section 3.3). tar.exe is
rem part of Windows and makes zip files.
set VERSION=
for /f "usebackq delims=" %%v in ("VERSION") do set VERSION=%%v
if "%VERSION%"=="" (echo No version in VERSION 1>&2 & exit /b 1)
set ZIP=target\Juku-%VERSION%-windows-x64.zip
if exist %ZIP% del %ZIP%
%SystemRoot%\System32\tar.exe -a -c -f %ZIP% -C %OUT% Juku.exe JukuThumbnail.dll || exit /b 1
echo Packed %ZIP%

rem The installer (frontends\windows\installer\Juku.iss), if Inno Setup 6 is
rem installed: ISCC.exe on the PATH, in ISCC, or where its installer puts it,
rem for the user or for all users.
if not defined ISCC for %%i in (ISCC.exe) do set "ISCC=%%~$PATH:i"
if not defined ISCC if exist "%LOCALAPPDATA%\Programs\Inno Setup 6\ISCC.exe" set "ISCC=%LOCALAPPDATA%\Programs\Inno Setup 6\ISCC.exe"
if not defined ISCC if exist "%ProgramFiles(x86)%\Inno Setup 6\ISCC.exe" set "ISCC=%ProgramFiles(x86)%\Inno Setup 6\ISCC.exe"
if not defined ISCC (echo Inno Setup 6 is not installed: no installer. 1>&2 & exit /b 0)
"%ISCC%" /Q "/DSource=%CD%\%OUT%" "/O%CD%\target" frontends\windows\installer\Juku.iss || exit /b 1
echo Made target\Juku-%VERSION%-windows-x64-setup.exe
