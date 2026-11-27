[Console]::OutputEncoding = [System.Text.Encoding]::UTF8
$summe = 0
foreach ($f in $args) {
  $n = (Get-Content -LiteralPath $f -ErrorAction SilentlyContinue | Measure-Object -Line).Lines
  $summe += $n
  "{0,8}  {1}" -f $n, (Split-Path $f -Leaf)
}
if ($args.Count -gt 1) { "{0,8}  zusammen" -f $summe }
