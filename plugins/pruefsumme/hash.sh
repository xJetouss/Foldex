for f in "$@"; do
  h=$(sha256sum "$f" | cut -d " " -f 1)
  echo "$h  $(basename "$f")"
done
