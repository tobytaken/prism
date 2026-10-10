#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

echo "Building prismd..."
cargo build --release --manifest-path "$ROOT_DIR/Cargo.toml" -p prismd

echo "Installing prismd binary..."
sudo install -Dm0755 \
  "$ROOT_DIR/target/release/prismd" \
  /usr/bin/prismd

echo "Installing systemd unit..."
sudo install -Dm0644 \
  "$ROOT_DIR/contrib/systemd/prismd.service" \
  /etc/systemd/system/prismd.service

if [[ ! -f /etc/prism/prismd.toml ]]; then
  echo "Installing default prismd config..."
  sudo install -Dm0644 \
    "$ROOT_DIR/contrib/prismd.toml" \
    /etc/prism/prismd.toml
else
  echo "Keeping existing /etc/prism/prismd.toml"
fi

echo "Reloading systemd and starting prismd..."
sudo systemctl daemon-reload
sudo systemctl enable --now prismd

echo "prismd is installed. Check status with: systemctl status prismd"
