#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
INSTALL_DIR="/opt/omtdecoder"

echo "== System update =="
sudo apt update

echo "== Ensure dependencies =="
sudo apt install -y clang git curl libasound2 libasound2-dev libdrm-dev pkg-config avahi-utils \
  libavcodec-dev libavutil-dev libswscale-dev

# Install Rust if not present
if ! command -v cargo >/dev/null 2>&1; then
  echo "== Install Rust =="
  curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
fi
# shellcheck disable=SC1091
[ -f "$HOME/.cargo/env" ] && source "$HOME/.cargo/env"

echo "== Build omtdecoder =="
cd "$ROOT_DIR"
cargo build --release

OMTPLAYER_BIN="$ROOT_DIR/target/release/omtdecoder"
if [[ ! -f "$OMTPLAYER_BIN" ]]; then
  echo "Build output not found: $OMTPLAYER_BIN"
  exit 1
fi

if [[ ! -f "$ROOT_DIR/omtdecoder/omtdecoder.service" ]]; then
  echo "Service file not found: $ROOT_DIR/omtdecoder/omtdecoder.service"
  exit 1
fi

echo "== Install service =="
if systemctl is-active --quiet omtdecoder; then
  sudo systemctl stop omtdecoder
fi
sudo mkdir -p "$INSTALL_DIR"
sudo cp "$OMTPLAYER_BIN" "$INSTALL_DIR"/
sudo cp "$ROOT_DIR/omtdecoder/omtdecoder.service" /etc/systemd/system/
sudo systemctl daemon-reload
sudo systemctl enable omtdecoder
sudo systemctl restart omtdecoder
sudo systemctl status omtdecoder --no-pager

echo "Done."
