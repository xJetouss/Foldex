[Console]::OutputEncoding = [System.Text.Encoding]::UTF8
$args | ForEach-Object { $h = Get-FileHash -Algorithm SHA256 -LiteralPath $_; "{0}  {1}" -f $h.Hash.ToLower(), (Split-Path $_ -Leaf) }
