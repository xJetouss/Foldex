d=$(date +%Y-%m-%d)
n=0
for f in "$@"; do
  name=$(basename "$f")
  dir=$(dirname "$f")
  case "$name" in
    *.*) ziel="$dir/${name%.*}_$d.${name##*.}" ;;
    *) ziel="$dir/${name}_$d" ;;
  esac
  cp "$f" "$ziel" && n=$((n+1))
done
echo "$n Kopie(n) mit Datum $d angelegt"
