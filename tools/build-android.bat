@echo off
rem Android-APK von Foldex bauen (Kern-App). Braucht: Android-SDK + NDK (z. B. über Android Studio),
rem   rustup target add aarch64-linux-android   und   cargo install cargo-apk
rem Ergebnis: dist\Foldex-Android.apk
setlocal
cd /d "%~dp0\.."
if not defined ANDROID_HOME set "ANDROID_HOME=%LOCALAPPDATA%\Android\Sdk"
if not exist "%ANDROID_HOME%" (
    echo FEHLER: Android-SDK nicht gefunden ^(%ANDROID_HOME%^).
    exit /b 1
)
if not defined ANDROID_NDK_ROOT for /d %%n in ("%ANDROID_HOME%\ndk\*") do set "ANDROID_NDK_ROOT=%%n"
if not defined JAVA_HOME if exist "%ProgramFiles%\Android\Android Studio\jbr" set "JAVA_HOME=%ProgramFiles%\Android\Android Studio\jbr"
set "PATH=%JAVA_HOME%\bin;%PATH%"
rem Signieren: eigener Schlüssel über CARGO_APK_RELEASE_KEYSTORE(_PASSWORD) – sonst der Debug-Schlüssel von
rem Android Studio (reicht zum Installieren auf dem eigenen Gerät, nicht für den Play Store)
if not defined CARGO_APK_RELEASE_KEYSTORE if exist "%USERPROFILE%\.android\debug.keystore" (
    set "CARGO_APK_RELEASE_KEYSTORE=%USERPROFILE%\.android\debug.keystore"
    set "CARGO_APK_RELEASE_KEYSTORE_PASSWORD=android"
)
rem Icons und Themes liegen im Hauptordner – fürs Android-Projekt verlinken
if not exist "android\assets" mklink /J "android\assets" "assets" >nul
set "VER="
for /f "tokens=3" %%v in ('findstr /b /c:"version = " Cargo.toml') do if not defined VER set "VER=%%~v"
pushd android
cargo apk build --release
if errorlevel 1 (
    popd
    echo FEHLER: APK konnte nicht gebaut werden.
    exit /b 1
)
popd
if not exist dist mkdir dist
copy /y "android\target\release\apk\Foldex.apk" "dist\Foldex-Android.apk" >nul || exit /b 1
echo Fertig: dist\Foldex-Android.apk
exit /b 0
