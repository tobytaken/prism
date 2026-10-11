#!/usr/bin/env bash
# Installs a prismd deb or rpm with the distro's package manager, as root in a throwaway container,
# and checks that the install created prismd's runtime directory from its tmpfiles.d entry.
set -euo pipefail

if [[ $# -ne 1 ]]; then
  echo "usage: $0 <prismd deb or rpm>" >&2
  exit 2
fi
package="$(realpath "$1")"

# A real host has systemd-tmpfiles, which the install scriptlet runs; a container image may not.
case "$package" in
  *.deb)
    apt-get update
    DEBIAN_FRONTEND=noninteractive apt-get install -y systemd
    rm -rf /run/prism
    DEBIAN_FRONTEND=noninteractive apt-get install -y "$package"
    ;;
  *.rpm)
    dnf install -y systemd
    rm -rf /run/prism
    dnf install -y "$package"
    ;;
  *)
    echo "not a deb or rpm: $package" >&2
    exit 2
    ;;
esac

failures=0
fail() {
  echo "$package: $1" >&2
  failures=$((failures + 1))
}

expected_entry="d /run/prism 0755 root root -"
grep -qxF "$expected_entry" /usr/lib/tmpfiles.d/prismd.conf \
  || fail "/usr/lib/tmpfiles.d/prismd.conf doesn't hold '$expected_entry'"
if [[ -d /run/prism ]]; then
  directory="$(stat -c '%a %U %G' /run/prism)"
  [[ "$directory" == "755 root root" ]] || fail "/run/prism is '$directory', expected '755 root root'"
else
  fail "the install didn't create /run/prism"
fi

if ((failures > 0)); then
  exit 1
fi
echo "$package: installs and creates /run/prism"
