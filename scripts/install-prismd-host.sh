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

echo "Installing prismd's runtime directory..."
sudo install -Dm0644 \
  "$ROOT_DIR/contrib/tmpfiles.d/prismd.conf" \
  /etc/tmpfiles.d/prismd.conf
sudo systemd-tmpfiles --create prismd.conf

if [[ ! -f /etc/prism/prismd.toml ]]; then
  echo "Installing default prismd config..."
  sudo install -Dm0644 \
    "$ROOT_DIR/contrib/prismd.toml" \
    /etc/prism/prismd.toml
else
  echo "Keeping existing /etc/prism/prismd.toml"
fi

# `enable --now` leaves a running prismd alone, so a reinstall would keep serving the old build.
# Restarting starts a stopped prismd too, and with Type=notify only returns once the new one is up.
if [[ "$(systemctl show -p MainPID --value prismd 2>/dev/null || true)" =~ ^[1-9] ]]; then
  outcome=restarted
  echo "Reloading systemd and restarting prismd so the new build takes effect..."
else
  outcome=started
  echo "Reloading systemd and starting prismd..."
fi
check_why="Check why with: systemctl status prismd, and journalctl -u prismd"

if ! sudo systemctl daemon-reload; then
  echo "Installed prismd's files, but systemd couldn't reload its units. $check_why" >&2
  exit 1
fi
if ! sudo systemctl enable prismd; then
  echo "Installed prismd's files, but couldn't enable prismd.service. $check_why" >&2
  exit 1
fi
if ! sudo systemctl restart prismd; then
  echo "Installed prismd's files, but prismd failed to start. $check_why" >&2
  exit 1
fi

echo "prismd is installed and $outcome. Check status with: systemctl status prismd"
