#!/usr/bin/env bash
# Writes the Flathub submission for a git ref (HEAD by default) to target/flathub: the Flatpak
# manifest, building from the release tag and its commit instead of this checkout, and the
# cargo-sources.json scripts/package-prism-flatpak.sh generated from the same Cargo.lock.
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
APP_ID="io.github.tobytaken.Prism"
REPO_URL="https://github.com/tobytaken/prism.git"
OUT_DIR="$ROOT_DIR/target/flathub"
ref="${1:-HEAD}"

cd "$ROOT_DIR"
if [[ ! -f contrib/flatpak/cargo-sources.json ]]; then
  echo "contrib/flatpak/cargo-sources.json is missing; run scripts/package-prism-flatpak.sh first" >&2
  exit 1
fi
# The tag the release workflow checks names this version.
version="$(bash scripts/workspace-version.sh "$ref")"
commit="$(git rev-parse "$ref^{commit}")"

rm -rf "$OUT_DIR"
mkdir -p "$OUT_DIR"
# Swaps the checkout's dir source, and the entries under it, for the tagged commit.
awk -v url="$REPO_URL" -v tag="v$version" -v commit="$commit" '
  skipping && match($0, /^ */) && RLENGTH > indent { next }
  { skipping = 0 }
  /^ *- type: dir$/ {
    indent = index($0, "-") - 1
    pad = sprintf("%" indent "s", "")
    print pad "- type: git"
    print pad "  url: " url
    print pad "  tag: " tag
    print pad "  commit: " commit
    skipping = 1
    replaced = 1
    next
  }
  { print }
  END { if (!replaced) exit 1 }
' "contrib/flatpak/$APP_ID.yml" >"$OUT_DIR/$APP_ID.yml" \
  || { echo "contrib/flatpak/$APP_ID.yml has no dir source to replace" >&2; exit 1; }
cp contrib/flatpak/cargo-sources.json "$OUT_DIR/"
echo "$OUT_DIR/$APP_ID.yml: v$version at $commit"
