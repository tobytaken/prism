#!/usr/bin/env bash
# Installs a Prism GUI deb or rpm with the distro's package manager, as root in a throwaway
# container, and checks that the installed prism finds every library it links.
set -euo pipefail

if [[ $# -ne 1 ]]; then
  echo "usage: $0 <prism-gpu deb or rpm>" >&2
  exit 2
fi
package="$(realpath "$1")"

case "$package" in
  *.deb)
    apt-get update
    DEBIAN_FRONTEND=noninteractive apt-get install -y "$package"
    ;;
  *.rpm) dnf install -y "$package" ;;
  *)
    echo "not a deb or rpm: $package" >&2
    exit 2
    ;;
esac

missing="$(ldd /usr/bin/prism | grep 'not found' || true)"
if [[ -n "$missing" ]]; then
  echo "$package: the installed prism is missing libraries:" >&2
  echo "$missing" >&2
  exit 1
fi
echo "$package: installs"
