# Einmalig als Administrator: das eigene Foldex-Zertifikat (von install-local.ps1 erzeugt) auf diesem PC
# als vertrauenswürdig für App-Pakete eintragen (Speicher „Vertrauenswürdige Personen“ des Computers).
# Es gilt nur für Pakete, die mit genau diesem Zertifikat signiert sind.
# Rückgängig: certlm.msc → Vertrauenswürdige Personen → „Foldex Local“ löschen.
$ErrorActionPreference = 'Stop'
$Cer = Join-Path $PSScriptRoot 'target\FoldexLocal.cer'
if (-not (Test-Path $Cer)) { throw "Zuerst install-local.ps1 ausführen ($Cer fehlt)." }
Import-Certificate -FilePath $Cer -CertStoreLocation Cert:\LocalMachine\TrustedPeople | Format-List Subject, Thumbprint, NotAfter
Write-Host 'Zertifikat eingetragen. Jetzt install-local.ps1 (ohne Administrator) erneut ausführen.' -ForegroundColor Green
