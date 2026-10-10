#!/usr/bin/env bash
# Packages an already built target/release/prism, which CI builds once on an old glibc, as an
# AppImage. It bundles no libraries: GL, Vulkan, the NVIDIA libraries and glibc come from the host.
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
APP_ID="io.github.tobytaken.Prism"
APPIMAGETOOL_URL="https://github.com/AppImage/appimagetool/releases/download/1.9.1/appimagetool-x86_64.AppImage"
APPIMAGETOOL_SHA256="ed4ce84f0d9caff66f50bcca6ff6f35aae54ce8135408b3fa33abfc3cb384eb0"
# The static type2 runtime, pinned rather than the latest one appimagetool would download.
RUNTIME_URL="https://github.com/AppImage/type2-runtime/releases/download/20251108/runtime-x86_64"
RUNTIME_SHA256="2fca8b443c92510f1483a883f60061ad09b46b978b2631c807cd873a47ec260d"

OUT_DIR="$ROOT_DIR/target/appimage"
TOOLS_DIR="$OUT_DIR/tools"
APPDIR="$OUT_DIR/Prism.AppDir"

download() {
  local url="$1" sha256="$2" file="$3"
  if [[ ! -f "$file" ]] || ! sha256sum --check --status <<<"$sha256  $file"; then
    curl --fail --silent --show-error --location --output "$file" "$url"
    sha256sum --check --quiet <<<"$sha256  $file"
  fi
}

cd "$ROOT_DIR"
mkdir -p "$TOOLS_DIR"
download "$APPIMAGETOOL_URL" "$APPIMAGETOOL_SHA256" "$TOOLS_DIR/appimagetool"
download "$RUNTIME_URL" "$RUNTIME_SHA256" "$TOOLS_DIR/runtime"
chmod +x "$TOOLS_DIR/appimagetool"

rm -rf "$APPDIR"
install -Dm755 target/release/prism "$APPDIR/usr/bin/prism"
install -Dm644 "contrib/$APP_ID.desktop" "$APPDIR/usr/share/applications/$APP_ID.desktop"
install -Dm644 "assets/$APP_ID.svg" "$APPDIR/usr/share/icons/hicolor/scalable/apps/$APP_ID.svg"
install -Dm644 LICENSE THIRD-PARTY-LICENSES -t "$APPDIR/usr/share/licenses/$APP_ID"
ln -s "usr/share/applications/$APP_ID.desktop" "$APPDIR/$APP_ID.desktop"
ln -s "usr/share/icons/hicolor/scalable/apps/$APP_ID.svg" "$APPDIR/$APP_ID.svg"
ln -s "$APP_ID.svg" "$APPDIR/.DirIcon"
ln -s usr/bin/prism "$APPDIR/AppRun"

# Extracting itself lets appimagetool run without FUSE, as in CI containers.
APPIMAGE_EXTRACT_AND_RUN=1 ARCH=x86_64 "$TOOLS_DIR/appimagetool" \
  --runtime-file "$TOOLS_DIR/runtime" "$APPDIR" "$OUT_DIR/Prism-x86_64.AppImage"
