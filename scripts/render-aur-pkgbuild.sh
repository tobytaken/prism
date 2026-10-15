#!/usr/bin/env bash
# Makes the AUR's split PKGBUILD for a git ref (HEAD by default) in target/aur: the source tarball
# the release attaches, with Cargo.lock, and contrib/aur's PKGBUILD with that version and the
# tarball's checksum. makepkg builds from there and finds the tarball without downloading it.
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
OUT_DIR="$ROOT_DIR/target/aur"
ref="${1:-HEAD}"

cd "$ROOT_DIR"
version="$(bash scripts/workspace-version.sh "$ref")"
tarball="prism-$version.tar.gz"

rm -rf "$OUT_DIR"
mkdir -p "$OUT_DIR"
git archive --format=tar.gz --prefix="prism-$version/" --output="$OUT_DIR/$tarball" "$ref"
sha256="$(sha256sum "$OUT_DIR/$tarball" | cut -d ' ' -f 1)"

sed -e "s/^pkgver=.*/pkgver=$version/" -e "s/^sha256sums=.*/sha256sums=('$sha256')/" \
  contrib/aur/PKGBUILD >"$OUT_DIR/PKGBUILD"
cp contrib/aur/prismd.install "$OUT_DIR/"
if ! grep -qxF "pkgver=$version" "$OUT_DIR/PKGBUILD" \
  || ! grep -qxF "sha256sums=('$sha256')" "$OUT_DIR/PKGBUILD"; then
  echo "contrib/aur/PKGBUILD has no pkgver or sha256sums line to fill in" >&2
  exit 1
fi
echo "$OUT_DIR/PKGBUILD: prism $version from $(git rev-parse "$ref^{commit}")"
