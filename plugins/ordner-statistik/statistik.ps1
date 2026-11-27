[Console]::OutputEncoding = [System.Text.Encoding]::UTF8
$ziel = if ($args.Count -gt 1 -and $args[1]) { $args[1] } else { $args[0] }
$f = Get-ChildItem -LiteralPath $ziel -File -Recurse -ErrorAction SilentlyContinue
"Ordner: $ziel"
"Dateien: $($f.Count)   Größe: {0:N1} MB" -f (($f | Measure-Object Length -Sum).Sum / 1MB)
""
$f | Group-Object { if ($_.Extension) { $_.Extension.ToLower() } else { "(ohne)" } } | Sort-Object Count -Descending | Select-Object -First 20 | ForEach-Object { "{0,6}  {1}" -f $_.Count, $_.Name }
