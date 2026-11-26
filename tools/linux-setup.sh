#!/bin/bash
# Einmalig in WSL (Ubuntu) ausführen: Werkzeuge für den Linux-Build von Foldex.
#   wsl -d Ubuntu -u root bash tools/linux-setup.sh
set -e
export DEBIAN_FRONTEND=noninteractive
apt-get update -q
apt-get install -y -q build-essential pkg-config cmake curl git clang \
    libgtk-3-dev libxdo-dev libasound2-dev libxkbcommon-dev libwayland-dev \
    libssl-dev libfontconfig1-dev libudev-dev libxkbcommon-x11-0
if [ ! -x "$HOME/.cargo/bin/cargo" ]; then
    curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal
fi
"$HOME/.cargo/bin/rustc" --version
