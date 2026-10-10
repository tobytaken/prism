#!/usr/bin/env bash
# Checks a built Prism GUI package: what it installs, the deb and rpm's Recommends, the Flatpak's
# sandbox permissions, and that the AppImage bundles none of the host's graphics stack or glibc.
#
# Usage: check-prism-package.sh deb|rpm|appimage <package>
#        check-prism-package.sh flatpak <flatpak-builder build directory>
set -euo pipefail

APP_ID="io.github.tobytaken.Prism"
# The glibc of AlmaLinux 8, where CI builds the release binary every package but the Flatpak repackages.
BASELINE_GLIBC="2.28"
# What the release binary may link, so the deb and rpm dependencies cover it: glibc, libgcc_s and
# fontconfig.
ALLOWED_LIBRARIES="ld-linux-x86-64.so.2 libc.so.6 libdl.so.2 libgcc_s.so.1 libm.so.6 libpthread.so.0 librt.so.1 libfontconfig.so.1"

if [[ $# -ne 2 ]]; then
  echo "usage: $0 deb|rpm|appimage|flatpak <package>" >&2
  exit 2
fi
kind="$1"
package="$(realpath "$2")"
failures=0

fail() {
  echo "$2: $1" >&2
  failures=$((failures + 1))
}

# Fails for each path that isn't in the package's file list on stdin.
expect_files() {
  local files
  files="$(cat)"
  for path in "$@"; do
    grep -qxF "$path" <<<"$files" || fail "doesn't install $path" "$package"
  done
}

expect_equal() {
  [[ "$2" == "$3" ]] || fail "$1 is '$2', expected '$3'" "$package"
}

# Fails when the packaged binary needs a newer glibc than the baseline or links a library the
# packages don't depend on.
check_binary() {
  local newest library
  newest="$(objdump -T "$1" | grep -oE 'GLIBC_[0-9.]+' | sed 's/GLIBC_//' | sort -Vu | tail -n 1)"
  [[ "$(printf '%s\n' "$newest" "$BASELINE_GLIBC" | sort -V | tail -n 1)" == "$BASELINE_GLIBC" ]] \
    || fail "prism needs glibc $newest, newer than the $BASELINE_GLIBC baseline" "$package"
  for library in $(objdump -p "$1" | awk '$1 == "NEEDED" { print $2 }'); do
    [[ " $ALLOWED_LIBRARIES " == *" $library "* ]] \
      || fail "prism links $library, which the packages don't depend on" "$package"
  done
}

extracted="$(mktemp -d)"
trap 'rm -rf "$extracted"' EXIT

case "$kind" in
  deb)
    expect_equal "Package" "$(dpkg-deb --field "$package" Package)" "prism-gpu"
    expect_equal "Recommends" "$(dpkg-deb --field "$package" Recommends)" "prismd"
    [[ "$(dpkg-deb --field "$package" Depends)" == *"libc6 (>= $BASELINE_GLIBC)"* ]] \
      || fail "Depends doesn't ask for glibc $BASELINE_GLIBC" "$package"
    dpkg-deb --extract "$package" "$extracted"
    check_binary "$extracted/usr/bin/prism"
    dpkg-deb --fsys-tarfile "$package" | tar -t | sed 's|^\./|/|' | expect_files \
      /usr/bin/prism \
      "/usr/share/applications/$APP_ID.desktop" \
      "/usr/share/icons/hicolor/scalable/apps/$APP_ID.svg" \
      /usr/share/doc/prism-gpu/copyright \
      /usr/share/doc/prism-gpu/THIRD-PARTY-LICENSES
    # Debian keeps the license in the copyright file.
    dpkg-deb --fsys-tarfile "$package" | tar -xO ./usr/share/doc/prism-gpu/copyright 2>/dev/null \
      | grep -qF "GNU GENERAL PUBLIC LICENSE" || fail "copyright doesn't hold the GPL" "$package"
    ;;
  rpm)
    expect_equal "Name" "$(rpm -qp --queryformat '%{NAME}' "$package")" "prism-gpu"
    expect_equal "License" "$(rpm -qp --queryformat '%{LICENSE}' "$package")" "GPL-3.0-or-later"
    expect_equal "Recommends" "$(rpm -qp --recommends "$package")" "prismd"
    (cd "$extracted" && rpm2cpio "$package" | cpio -idm --quiet ./usr/bin/prism)
    check_binary "$extracted/usr/bin/prism"
    rpm -qlp "$package" | expect_files \
      /usr/bin/prism \
      "/usr/share/applications/$APP_ID.desktop" \
      "/usr/share/icons/hicolor/scalable/apps/$APP_ID.svg" \
      /usr/share/licenses/prism-gpu/LICENSE \
      /usr/share/licenses/prism-gpu/THIRD-PARTY-LICENSES
    ;;
  appimage)
    (cd "$extracted" && "$package" --appimage-extract >/dev/null)
    root="$extracted/squashfs-root"
    (cd "$root" && find . -mindepth 1 | sed 's|^\./|/|') | expect_files \
      /AppRun \
      "/$APP_ID.desktop" \
      "/$APP_ID.svg" \
      /usr/bin/prism \
      "/usr/share/applications/$APP_ID.desktop" \
      "/usr/share/icons/hicolor/scalable/apps/$APP_ID.svg" \
      "/usr/share/licenses/$APP_ID/LICENSE" \
      "/usr/share/licenses/$APP_ID/THIRD-PARTY-LICENSES"
    check_binary "$root/usr/bin/prism"
    # These must come from the host: its GPU driver's GL, Vulkan and NVIDIA libraries, and glibc.
    bundled="$(find "$root" \( -type f -o -type l \) \( \
      -name 'libGL*' -o -name 'libEGL*' -o -name 'libOpenGL*' -o -name 'libvulkan*' \
      -o -iname '*nvidia*' -o -name 'libnv*' -o -name 'libcuda*' \
      -o -name 'ld-linux*' -o -name 'libc.so*' -o -name 'libc-*' -o -name 'libm.so*' \
      -o -name 'libdl.so*' -o -name 'libpthread.so*' -o -name 'librt.so*' \
      -o -name 'libresolv.so*' -o -name 'libutil.so*' \) -printf '%P\n')"
    [[ -z "$bundled" ]] || fail "bundles libraries the host must provide: $bundled" "$package"
    ;;
  flatpak)
    (cd "$package/files" && find . -mindepth 1 | sed 's|^\./|/app/|') | expect_files \
      /app/bin/prism \
      "/app/share/applications/$APP_ID.desktop" \
      "/app/share/icons/hicolor/scalable/apps/$APP_ID.svg" \
      "/app/share/licenses/$APP_ID/LICENSE" \
      "/app/share/licenses/$APP_ID/THIRD-PARTY-LICENSES"
    # Every permission the sandbox gets, one per line, so an extra one fails as much as a missing one.
    permissions="$(awk '
      /^\[/ { section = $0; next }
      section == "[Context]" || section ~ /Bus Policy\]$/ {
        split($0, entry, "=")
        if (section == "[Context]") {
          count = split(entry[2], values, ";")
          for (i = 1; i <= count; i++) if (values[i] != "") print section " " entry[1] "=" values[i]
        } else if ($0 != "") print section " " $0
      }' "$package/metadata" | sort)"
    expected="$(sort <<'EOF'
[Context] devices=dri
[Context] filesystems=/run/prism
[Context] shared=ipc
[Context] sockets=fallback-x11
[Context] sockets=wayland
[Session Bus Policy] org.freedesktop.Notifications=talk
EOF
)"
    [[ "$permissions" == "$expected" ]] \
      || fail "has the permissions"$'\n'"$permissions"$'\n'"expected"$'\n'"$expected" "$package"
    ;;
  *)
    echo "unknown package kind: $kind" >&2
    exit 2
    ;;
esac

if ((failures > 0)); then
  exit 1
fi
echo "$package: ok"
