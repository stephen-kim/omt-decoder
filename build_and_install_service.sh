#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
LIBVMX_DIR="$ROOT_DIR/libvmx"
LIBOMTNET_DIR="$ROOT_DIR/libomtnet"
PLAYER_DIR="$ROOT_DIR/omtplayer"
BUILD_DIR="$PLAYER_DIR/build/arm64"
INSTALL_DIR="/opt/omtplayer"

echo "== System update =="
sudo apt update

echo "== Ensure dependencies =="
sudo apt install -y clang git curl libasound2

if ! command -v dotnet >/dev/null 2>&1; then
  echo "== Install dotnet 8 =="
  curl -sSL https://dot.net/v1/dotnet-install.sh | bash /dev/stdin --channel 8.0
  if ! grep -q "DOTNET_ROOT" "$HOME/.bashrc"; then
    echo 'export DOTNET_ROOT=$HOME/.dotnet' >> "$HOME/.bashrc"
    echo 'export PATH=$PATH:$HOME/.dotnet' >> "$HOME/.bashrc"
  fi
  # shellcheck disable=SC1090
  source "$HOME/.bashrc"
fi

if [[ ! -d "$LIBVMX_DIR/build" || ! -d "$LIBOMTNET_DIR/build" || ! -d "$PLAYER_DIR/build" ]]; then
  echo "Build directories not found. Expected libvmx/libomtnet/omtplayer under: $ROOT_DIR"
  exit 1
fi

echo "== Clean previous builds (if any) =="
rm -rf "$LIBVMX_DIR/build/arm64" "$LIBOMTNET_DIR/build/arm64" "$PLAYER_DIR/build/arm64"

echo "== Build libvmx =="
chmod 755 "$LIBVMX_DIR/build/buildlinuxarm64.sh"
(cd "$LIBVMX_DIR/build" && ./buildlinuxarm64.sh)

echo "== Build libomtnet =="
chmod 755 "$LIBOMTNET_DIR/build/buildall.sh"
(cd "$LIBOMTNET_DIR/build" && ./buildall.sh)

echo "== Build omtplayer =="
chmod 755 "$PLAYER_DIR/build/buildlinuxarm64.sh"
(cd "$PLAYER_DIR/build" && ./buildlinuxarm64.sh)

if [[ ! -d "$BUILD_DIR" ]]; then
  echo "Build output not found: $BUILD_DIR"
  exit 1
fi

if [[ ! -f "$PLAYER_DIR/omtplayer.service" ]]; then
  echo "Service file not found: $PLAYER_DIR/omtplayer.service"
  exit 1
fi

echo "== Install service =="
sudo mkdir -p "$INSTALL_DIR"
sudo cp "$BUILD_DIR"/* "$INSTALL_DIR"/
sudo cp "$PLAYER_DIR/omtplayer.service" /etc/systemd/system/
sudo systemctl daemon-reload
sudo systemctl enable omtplayer
sudo systemctl restart omtplayer
sudo systemctl status omtplayer --no-pager

echo "Done."
