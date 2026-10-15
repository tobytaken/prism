#!/usr/bin/env bash
# Installs a Prism GUI deb, rpm or Arch package with the distro's package manager, as root in a
# throwaway container, and checks that the installed prism finds every library it links or loads
# at runtime.
set -euo pipefail

if [[ $# -ne 1 ]]; then
  echo "usage: $0 <prism-gpu deb, rpm or pkg.tar.zst>" >&2
  exit 2
fi
package="$(realpath "$1")"

case "$package" in
  *.deb)
    apt-get update
    DEBIAN_FRONTEND=noninteractive apt-get install -y "$package"
    ;;
  *.rpm) dnf install -y "$package" ;;
  *.pkg.tar.zst)
    pacman -Syu --noconfirm
    pacman -U --noconfirm "$package"
    ;;
  *)
    echo "not a deb, rpm or pkg.tar.zst: $package" >&2
    exit 2
    ;;
esac

missing="$(ldd /usr/bin/prism | grep 'not found' || true)"
if [[ -n "$missing" ]]; then
  echo "$package: the installed prism is missing libraries:" >&2
  echo "$missing" >&2
  exit 1
fi
# Slint loads its windowing and GL libraries at runtime, so ldd doesn't see them; the binary names
# them. NVML comes with the NVIDIA driver, not the package's dependencies.
installed="$(ldconfig -p | awk 'NR > 1 { print $1 }')"
while read -r library; do
  if [[ "$library" != libnvidia-ml.so.1 ]] && ! grep -qxF "$library" <<<"$installed"; then
    echo "$package: the installed prism loads $library, which isn't installed" >&2
    exit 1
  fi
done < <(grep -aoE 'lib[A-Za-z0-9_+-]+\.so\.[0-9]+' /usr/bin/prism | sort -u)
echo "$package: installs"
