#!/bin/bash
# Linux-Version von Foldex unter Windows bauen (in WSL/Ubuntu). Aufruf aus build.bat:
#   wsl -d Ubuntu -u root bash /mnt/e/Rust/Foldex/tools/build-linux.sh /mnt/e/Rust/Foldex
# Ergebnis: dist/Foldex-Linux-x86_64.tar.gz (Programm + Startmenü-Eintrag über --install-desktop)
set -e
SRC="${1:-/mnt/e/Rust/Foldex}"
WORK="$HOME/foldex-build"
export PATH="$HOME/.cargo/bin:$PATH"
mkdir -p "$WORK"
# Quellcode ins Linux-Dateisystem kopieren (Bauen direkt auf /mnt/… ist sehr langsam)
for d in src assets installer; do
    rm -rf "$WORK/$d"
    [ -d "$SRC/$d" ] && cp -r "$SRC/$d" "$WORK/$d"
done
cp "$SRC/Cargo.toml" "$SRC/build.rs" "$WORK/"
[ -f "$SRC/Cargo.lock" ] && cp "$SRC/Cargo.lock" "$WORK/"
cd "$WORK"
VER=$(grep -m1 '^version' Cargo.toml | cut -d'"' -f2)
cargo build --release
NAME="Foldex-Linux-x86_64"
rm -rf "/tmp/$NAME" && mkdir -p "/tmp/$NAME"
cp target/release/foldex "/tmp/$NAME/foldex"
strip "/tmp/$NAME/foldex" || true
cat > "/tmp/$NAME/LIESMICH.txt" <<EOF
Foldex $VER für Linux (x86_64)

Starten:            ./foldex
Ins Startmenü:      ./foldex --install-desktop
Benötigt: GTK 3, ALSA und libxkbcommon-x11 (auf Desktop-Distributionen meist schon vorhanden;
Ubuntu/Debian: sudo apt install libgtk-3-0 libasound2 libxkbcommon-x11-0).
EOF
tar -czf "$SRC/dist/$NAME.tar.gz" -C /tmp "$NAME"
echo "Fertig: dist/$NAME.tar.gz"
