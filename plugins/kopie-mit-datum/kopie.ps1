[Console]::OutputEncoding = [System.Text.Encoding]::UTF8
$d = Get-Date -Format "yyyy-MM-dd"
$n = 0
foreach ($f in $args) {
  $i = Get-Item -LiteralPath $f
  $ziel = Join-Path $i.DirectoryName ("{0}_{1}{2}" -f $i.BaseName, $d, $i.Extension)
  Copy-Item -LiteralPath $f -Destination $ziel
  $n++
}
"$n Kopie(n) mit Datum $d angelegt"
