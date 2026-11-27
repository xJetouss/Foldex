ziel="${2:-$1}"
[ -z "$ziel" ] && ziel="$1"
echo "Ordner: $ziel"
echo "Dateien: $(find "$ziel" -type f 2>/dev/null | wc -l)   Größe: $(du -sh "$ziel" 2>/dev/null | cut -f 1)"
echo
find "$ziel" -type f 2>/dev/null | sed -n "s/.*\/[^/]*\.\([^./]*\)$/.\1/p" | tr "A-Z" "a-z" | sort | uniq -c | sort -rn | head -n 20
