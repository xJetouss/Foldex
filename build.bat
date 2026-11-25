@echo off
rem Foldex bauen – alles landet im Ordner dist:
rem   dist\foldex.exe            die normale EXE (startet direkt, ohne Installation)
rem   dist\Foldex-Installer.exe  Windows-Installer mit Assistent (Ort, Verknuepfungen, KI-Komponenten einzeln waehlbar)
rem   dist\foldex-<version>.tar.gz  Quellcode-Paket fuer Linux (z. B. Arch Linux)
rem Der Installer ueberschreibt eine bereits installierte (alte) Version automatisch.
rem Aufruf: Doppelklick oder "build.bat" im Projektordner.
setlocal
cd /d "%~dp0"

set "DIST=dist"
set "SETUP=%DIST%\Foldex-Installer.exe"
set "VERCS=%TEMP%\foldex_installer_version.cs"
set "CSC=%WINDIR%\Microsoft.NET\Framework64\v4.0.30319\csc.exe"

rem Version aus Cargo.toml lesen (Zeile: version = "0.5.0" -> 3. Token)
set "VER="
for /f "tokens=3" %%v in ('findstr /b /c:"version = " Cargo.toml') do if not defined VER set "VER=%%~v"
if not defined VER (
    echo FEHLER: Version in Cargo.toml nicht gefunden.
    pause
    exit /b 1
)

echo [1/4] Foldex wird gebaut (Release) ...
cargo build --release
if errorlevel 1 (
    echo.
    echo Bauen fehlgeschlagen - siehe Meldungen oben.
    pause
    exit /b 1
)

echo [2/4] dist-Ordner wird vorbereitet ...
if exist "%DIST%" rmdir /s /q "%DIST%"
if not exist "%DIST%" mkdir "%DIST%" || goto :err
rem die normale EXE (startet direkt, ohne Installation)
copy /y "target\release\foldex.exe" "%DIST%\foldex.exe" >nul || goto :err

echo [3/4] Windows-Installer (Foldex-Installer.exe) wird erstellt ...
rem Quelltext: installer\Installer.cs (C#, mit dem bei Windows mitgelieferten Compiler)
>"%VERCS%" echo static class Ver { public const string Value = "%VER%"; }
if not exist "%CSC%" set "CSC=%WINDIR%\Microsoft.NET\Framework\v4.0.30319\csc.exe"
if not exist "%CSC%" goto :cscfehler
"%CSC%" /nologo /target:winexe /r:System.Windows.Forms.dll /r:System.Drawing.dll /r:System.Web.Extensions.dll /win32icon:"%CD%\assets\app.ico" /resource:"%CD%\assets\app.ico",app.ico /out:"%SETUP%" "installer\Installer.cs" "%VERCS%"
if errorlevel 1 goto :cscfehler
del "%VERCS%" >nul 2>&1
rem Nutzdaten anhaengen: Markierung + Laenge (16 Hex-Zeichen) + foldex.exe
powershell -NoProfile -Command "$f=[IO.File]::Open('%CD%\dist\Foldex-Installer.exe','Append'); $b=[IO.File]::ReadAllBytes('%CD%\target\release\foldex.exe'); $m=[Text.Encoding]::ASCII.GetBytes('FOLDEX-PAYLOAD-BEGIN'); $f.Write($m,0,$m.Length); $f.Write([Text.Encoding]::ASCII.GetBytes($b.Length.ToString('X16')),0,16); $f.Write($b,0,$b.Length); $f.Close()"
if errorlevel 1 goto :err

echo [4/4] Quellcode-Paket (tar.gz) fuer Linux wird erstellt ...
where tar >nul 2>&1
if errorlevel 1 goto :tarfehler
set "STG=%TEMP%\foldex-%VER%"
if exist "%STG%" rmdir /s /q "%STG%"
xcopy /e /i /q /y "src" "%STG%\src" >nul || goto :err
if exist "assets" xcopy /e /i /q /y "assets" "%STG%\assets" >nul || goto :err
copy /y "build.rs" "%STG%\" >nul || goto :err
copy /y "Cargo.toml" "%STG%\" >nul || goto :err
copy /y "Cargo.lock" "%STG%\" >nul || goto :err
if exist "build.sh" copy /y "build.sh" "%STG%\" >nul
tar -czf "%DIST%\foldex-%VER%.tar.gz" -C "%TEMP%" "foldex-%VER%"
if errorlevel 1 goto :tarfehler
rmdir /s /q "%STG%"

echo.
echo Fertig! Alles liegt im Ordner dist:
echo   dist\foldex.exe              die normale EXE: startet direkt, ohne Installation
echo   dist\Foldex-Installer.exe    Windows-Installer: Assistent mit Installationsort, Verknuepfungen,
echo                                und KI-Komponenten zum Auswaehlen
echo                                (still: /silent [/dir=...] [/ai=id,id])
echo   dist\foldex-%VER%.tar.gz     Quellcode fuer Linux: entpacken, dann "sh build.sh"
pause
exit /b 0

:cscfehler
echo.
echo FEHLER: Der C#-Compiler (csc.exe) wurde nicht gefunden oder das Kompilieren ist fehlgeschlagen.
echo csc.exe ist bei Windows automatisch dabei (.NET Framework 4).
pause
exit /b 1

:tarfehler
echo.
echo FEHLER: tar.exe nicht gefunden oder das Quellcode-Paket konnte nicht erstellt werden.
echo tar.exe ist ab Windows 10 automatisch bei Windows dabei.
pause
exit /b 1

:err
echo.
echo FEHLER: build.bat konnte nicht fertig gestellt werden - siehe Meldungen oben.
pause
exit /b 1
