summe=0
for f in "$@"; do
  n=$(wc -l < "$f")
  summe=$((summe + n))
  printf "%8s  %s\n" "$n" "$(basename "$f")"
done
[ $# -gt 1 ] && printf "%8s  zusammen\n" "$summe"
