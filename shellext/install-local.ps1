# „Mit Foldex öffnen“ oben im neuen Kontextmenü von Windows 11 – nur für diesen PC.
#
# Baut die Shell-Erweiterung, legt sie neben die installierte foldex.exe, packt das Paket, signiert es mit einem
# eigenen Zertifikat (CN=Foldex Local, im Benutzer-Speicher) und meldet es an.
# Windows nimmt das Paket nur an, wenn dieses Zertifikat einmalig als vertrauenswürdig eingetragen ist –
# das macht trust-cert.ps1 (als Administrator).
#
#   powershell -ExecutionPolicy Bypass -File shellext\install-local.ps1            # einrichten / aktualisieren
#   powershell -ExecutionPolicy Bypass -File shellext\install-local.ps1 -Remove    # wieder entfernen
param([switch]$Remove)
$ErrorActionPreference = 'Stop'

$Here = $PSScriptRoot
$Install = Join-Path $env:LOCALAPPDATA 'Programs\Foldex'
$Name = 'Foldex.ContextMenu'
$Subject = 'CN=Foldex Local'
$Work = Join-Path $Here 'target\package'
$Cer = Join-Path $Here 'target\FoldexLocal.cer'

if ($Remove) {
    Get-AppxPackage $Name | Remove-AppxPackage
    Remove-Item (Join-Path $Install 'foldex_shell.dll') -ErrorAction SilentlyContinue
    Write-Host 'Entfernt.'
    return
}

if (-not (Test-Path (Join-Path $Install 'foldex.exe'))) { throw "Foldex ist nicht installiert ($Install)." }

# Werkzeuge aus dem Windows SDK
$Kit = Get-ChildItem 'C:\Program Files (x86)\Windows Kits\10\bin\10.*\x64\makeappx.exe' | Sort-Object FullName | Select-Object -Last 1
if (-not $Kit) { throw 'Windows SDK (makeappx.exe) nicht gefunden.' }
$MakeAppx = $Kit.FullName
$SignTool = Join-Path $Kit.DirectoryName 'signtool.exe'

# 1. DLL bauen und neben foldex.exe legen (eine geladene alte DLL vorher abmelden)
Push-Location $Here
try { cargo build --release; if ($LASTEXITCODE) { throw 'cargo build fehlgeschlagen' } } finally { Pop-Location }
Get-AppxPackage $Name | Remove-AppxPackage
Copy-Item (Join-Path $Here 'target\release\foldex_shell.dll') $Install -Force

# 2. Paket packen
Remove-Item $Work -Recurse -Force -ErrorAction SilentlyContinue
New-Item -ItemType Directory $Work | Out-Null
Copy-Item (Join-Path $Here 'AppxManifest.xml') $Work
Copy-Item (Join-Path $Here '..\web\img\logo.png') $Work
$Msix = Join-Path $Here 'target\foldex-shell.msix'
& $MakeAppx pack /o /nv /d $Work /p $Msix | Out-Null
if ($LASTEXITCODE) { throw 'makeappx fehlgeschlagen' }

# 3. Zertifikat (einmalig anlegen) und signieren
$Cert = Get-ChildItem Cert:\CurrentUser\My | Where-Object { $_.Subject -eq $Subject -and $_.NotAfter -gt (Get-Date) } | Select-Object -First 1
if (-not $Cert) {
    $Cert = New-SelfSignedCertificate -Type Custom -Subject $Subject -KeyUsage DigitalSignature -FriendlyName 'Foldex (nur dieser PC)' `
        -CertStoreLocation Cert:\CurrentUser\My -NotAfter (Get-Date).AddYears(10) `
        -TextExtension @('2.5.29.37={text}1.3.6.1.5.5.7.3.3', '2.5.29.19={text}')
}
Export-Certificate -Cert $Cert -FilePath $Cer | Out-Null
& $SignTool sign /fd SHA256 /sha1 $Cert.Thumbprint /s My $Msix | Out-Null
if ($LASTEXITCODE) { throw 'signtool fehlgeschlagen' }

# 4. anmelden
$Trusted = Get-ChildItem Cert:\LocalMachine\TrustedPeople | Where-Object Thumbprint -eq $Cert.Thumbprint
if (-not $Trusted) {
    Write-Host ''
    Write-Host 'Das Zertifikat ist noch nicht vertrauenswürdig. Einmalig als Administrator ausführen:' -ForegroundColor Yellow
    Write-Host "  powershell -ExecutionPolicy Bypass -File `"$(Join-Path $Here 'trust-cert.ps1')`""
    Write-Host 'und danach dieses Skript noch einmal.'
    exit 2
}
Add-AppxPackage -Path $Msix -ExternalLocation $Install
Write-Host 'Fertig: „Mit Foldex öffnen“ steht jetzt im Rechtsklick-Menü von Ordnern.' -ForegroundColor Green
