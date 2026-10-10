#!/usr/bin/env bash
# Builds the Flatpak from source per its manifest and bundles it. Needs flatpak-builder and Python
# with aiohttp and tomlkit, as in CI's ghcr.io/flathub-infra/flatpak-github-actions image.
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
APP_ID="io.github.tobytaken.Prism"
MANIFEST="contrib/flatpak/$APP_ID.yml"
GENERATOR_URL="https://raw.githubusercontent.com/flatpak/flatpak-builder-tools/74697c75b630d7330e77250fc13cb5ea688d9479/cargo/flatpak-cargo-generator.py"
GENERATOR_SHA256="0a2db6be87d75910facef28ab46d4d6460802e8419ab850d0caa6a364d26b380"

OUT_DIR="$ROOT_DIR/target/flatpak"

cd "$ROOT_DIR"
mkdir -p "$OUT_DIR"
generator="$OUT_DIR/flatpak-cargo-generator.py"
if [[ ! -f "$generator" ]] || ! sha256sum --check --status <<<"$GENERATOR_SHA256  $generator"; then
  curl --fail --silent --show-error --location --output "$generator" "$GENERATOR_URL"
  sha256sum --check --quiet <<<"$GENERATOR_SHA256  $generator"
fi
# The vendored crates, generated from Cargo.lock, so the build runs offline.
python3 "$generator" Cargo.lock --output contrib/flatpak/cargo-sources.json

flatpak remote-add --user --if-not-exists flathub https://dl.flathub.org/repo/flathub.flatpakrepo
flatpak-builder --user --install-deps-from=flathub --disable-rofiles-fuse --force-clean \
  --state-dir="$OUT_DIR/state" --repo="$OUT_DIR/repo" "$OUT_DIR/build" "$MANIFEST"
flatpak build-bundle "$OUT_DIR/repo" "$OUT_DIR/prism.flatpak" "$APP_ID" \
  --runtime-repo=https://dl.flathub.org/repo/flathub.flatpakrepo
