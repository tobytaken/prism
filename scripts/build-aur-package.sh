#!/usr/bin/env bash
# Builds target/aur's PKGBUILD (scripts/render-aur-pkgbuild.sh) with makepkg, as root in a clean Arch
# container, and writes the .SRCINFO the AUR needs beside it. makepkg won't run as root, so a
# throwaway user builds, with sudo for pacman to install the makedepends.
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
AUR_DIR="$ROOT_DIR/target/aur"
BUILD_DIR="/tmp/aur-build"

pacman -Syu --noconfirm --needed base-devel sudo
id builder >/dev/null 2>&1 || useradd --create-home builder
echo 'builder ALL=(root) NOPASSWD: /usr/bin/pacman' >/etc/sudoers.d/builder

rm -rf "$BUILD_DIR"
cp -r "$AUR_DIR" "$BUILD_DIR"
chown -R builder "$BUILD_DIR"
(
  cd "$BUILD_DIR"
  sudo -u builder makepkg --syncdeps --noconfirm
  # Root writes the file; only makepkg must not run as root.
  # shellcheck disable=SC2024
  sudo -u builder makepkg --printsrcinfo >.SRCINFO
)
cp "$BUILD_DIR"/*.pkg.tar.zst "$BUILD_DIR/.SRCINFO" "$AUR_DIR/"
ls -l "$AUR_DIR"
