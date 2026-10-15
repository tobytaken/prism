#!/usr/bin/env bash
# Publishes the GitHub release for a tag from the CI run's artifacts, downloaded into one directory
# with a subdirectory per artifact: the deb and rpm packages, the AppImage and Flatpak bundle, the
# source tarball the AUR PKGBUILD builds from, the AUR and Flathub files, and their checksums. The
# notes name the commit the release is built from.
#
# Usage: publish-release.sh <tag> <artifacts directory>
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
APP_ID="io.github.tobytaken.Prism"

if [[ $# -ne 2 ]]; then
  echo "usage: $0 <tag> <artifacts directory>" >&2
  exit 2
fi
tag="$1"
artifacts="$(realpath "$2")"
version="${tag#v}"
commit="$(git -C "$ROOT_DIR" rev-parse "$tag^{commit}")"

assets="$(mktemp -d)"
trap 'rm -rf "$assets"' EXIT
cp "$artifacts"/prism-packages/debian/prism-gpu_*.deb "$artifacts"/prismd-packages/debian/prismd_*.deb \
  "$artifacts"/prism-packages/generate-rpm/prism-gpu-*.rpm \
  "$artifacts"/prismd-packages/generate-rpm/prismd-*.rpm \
  "$artifacts/prism-packages/appimage/Prism-x86_64.AppImage" "$assets/"
cp "$artifacts/prism-flatpak/prism.flatpak" "$assets/$APP_ID.flatpak"
cp "$artifacts/aur/prism-$version.tar.gz" "$assets/"
tar -C "$artifacts/aur" -czf "$assets/prism-gpu-aur-$version.tar.gz" PKGBUILD prismd.install .SRCINFO
tar -C "$artifacts/flathub" -czf "$assets/$APP_ID-flathub-$version.tar.gz" "$APP_ID.yml" cargo-sources.json
(cd "$assets" && sha256sum -- * >SHA256SUMS)

notes="Built from \`$tag\` at commit $commit.

- \`prism-$version.tar.gz\`: the source at that commit, with \`Cargo.lock\`. The AUR PKGBUILD builds from it.
- \`prism-gpu\` (the GUI) and \`prismd\` (the daemon) as deb and rpm packages. The \`prismd\` deb enables and starts the service; the rpm follows the distribution's presets.
- \`Prism-x86_64.AppImage\` and \`$APP_ID.flatpak\`: the GUI only. Tuning needs \`prismd\` installed on the host from its deb, rpm or AUR package.
- \`prism-gpu-aur-$version.tar.gz\`: the AUR's \`PKGBUILD\`, \`prismd.install\` and \`.SRCINFO\`.
- \`$APP_ID-flathub-$version.tar.gz\`: the Flathub manifest and \`cargo-sources.json\`.
- \`SHA256SUMS\`: the checksums of all of these."

gh release create "$tag" --verify-tag --title "Prism $version" --notes "$notes" "$assets"/*
