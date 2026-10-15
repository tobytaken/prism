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
if systemctl is-active --quiet prismd; then
  action=restart
  done_action=restarted
  progress="Restarting prismd so the new build takes effect..."
else
  action=start
  done_action=started
  progress="Starting prismd..."
fi
diagnose="See: systemctl status prismd, and journalctl -u prismd"

echo "Reloading systemd..."
sudo systemctl daemon-reload

echo "Enabling prismd for future boots..."
if ! sudo systemctl enable prismd; then
  echo "Installed prismd's files, but couldn't enable prismd.service. $diagnose" >&2
  exit 1
fi

echo "$progress"
if ! sudo systemctl restart prismd; then
  echo "prismd didn't $action. $diagnose" >&2
  exit 1
fi

echo "prismd is installed and $done_action. Check status with: systemctl status prismd"
