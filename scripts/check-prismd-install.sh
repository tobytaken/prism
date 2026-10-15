#!/usr/bin/env bash
# Installs a prismd deb, rpm or Arch package with the distro's package manager, as root in a
# throwaway container, and checks what the install leaves: /run/prism from the tmpfiles.d entry,
# the license and third-party notices, a config file upgrades keep, and the service. The deb enables
# and starts prismd, the rpm follows the presets, and the Arch package leaves it disabled (#99).
# Then it removes the package and checks the service went with it.
#
# Booted with systemd (scripts/run-in-systemd-container.sh), it also checks whether prismd runs;
# otherwise only whether it's enabled.
#
# Usage: check-prismd-install.sh <prismd deb, rpm or pkg.tar.zst>
set -euo pipefail

if [[ $# -ne 1 ]]; then
  echo "usage: $0 <prismd deb, rpm or pkg.tar.zst>" >&2
  exit 2
fi
package="$(realpath "$1")"
UNIT="prismd.service"
UNIT_FILE="/usr/lib/systemd/system/$UNIT"
LICENSE="GPL-3.0-or-later"
# A preset this script adds for the rpm's second install, so its first follows the distro's own.
PRESET="/etc/systemd/system-preset/00-prismd-check.preset"

failures=0
fail() {
  echo "$package: $1" >&2
  failures=$((failures + 1))
}

booted() {
  [[ -d /run/systemd/system ]]
}

# Checks the unit is enabled or not, and, booted, whether it runs.
expect_service() {
  local enabled="$1" active="$2" state
  state="$(systemctl is-enabled "$UNIT" 2>/dev/null || true)"
  [[ "$state" == "$enabled" ]] || fail "$UNIT is '$state', expected '$enabled'"
  if booted; then
    state="$(systemctl is-active "$UNIT" 2>/dev/null || true)"
    [[ "$state" == "$active" ]] || fail "$UNIT is '$state', expected '$active'"
  fi
}

# After removal: no unit, nothing enabled and, booted, nothing running.
expect_service_removed() {
  [[ ! -e "$UNIT_FILE" ]] || fail "removing it leaves $UNIT"
  ! systemctl is-enabled --quiet "$UNIT" 2>/dev/null || fail "removing it leaves $UNIT enabled"
  if booted && systemctl is-active --quiet "$UNIT"; then
    fail "removing it leaves $UNIT running"
  fi
}

expect_file() {
  [[ -f "$1" ]] || fail "doesn't install $1"
}

# The tmpfiles.d entry and the /run/prism it creates; a container image may lack the
# systemd-tmpfiles that a real host's install runs.
expect_runtime_directory() {
  local expected_entry="d /run/prism 0755 root root -" directory
  grep -qxF "$expected_entry" /usr/lib/tmpfiles.d/prismd.conf \
    || fail "/usr/lib/tmpfiles.d/prismd.conf doesn't hold '$expected_entry'"
  if [[ -d /run/prism ]]; then
    directory="$(stat -c '%a %U %G' /run/prism)"
    [[ "$directory" == "755 root root" ]] || fail "/run/prism is '$directory', expected '755 root root'"
  else
    fail "the install didn't create /run/prism"
  fi
}

case "$package" in
  *.deb)
    apt-get update
    DEBIAN_FRONTEND=noninteractive apt-get install -y systemd
    # Docker's Ubuntu image leaves out what packages install under /usr/share/doc; a host doesn't.
    rm -f /etc/dpkg/dpkg.cfg.d/excludes
    rm -rf /run/prism
    DEBIAN_FRONTEND=noninteractive apt-get install -y "$package"
    expect_runtime_directory
    expect_file "$UNIT_FILE"
    # Debian keeps the license in the copyright file, with the license field in its header.
    grep -qxF "License: $LICENSE" /usr/share/doc/prismd/copyright \
      || fail "copyright doesn't name $LICENSE"
    grep -qF "GNU GENERAL PUBLIC LICENSE" /usr/share/doc/prismd/copyright \
      || fail "copyright doesn't hold the GPL"
    expect_file /usr/share/doc/prismd/THIRD-PARTY-LICENSES
    dpkg-query --show --showformat='${Conffiles}\n' prismd | grep -qF " /etc/prism/prismd.toml " \
      || fail "/etc/prism/prismd.toml isn't a conffile"
    expect_service enabled active

    DEBIAN_FRONTEND=noninteractive apt-get purge -y prismd
    expect_service_removed
    ;;
  *.rpm)
    dnf install -y systemd
    rm -rf /run/prism "$PRESET"
    dnf install -y "$package"
    expect_runtime_directory
    expect_file "$UNIT_FILE"
    expect_file /usr/share/licenses/prismd/LICENSE
    expect_file /usr/share/licenses/prismd/THIRD-PARTY-LICENSES
    license="$(rpm -q --queryformat '%{LICENSE}' prismd)"
    [[ "$license" == "$LICENSE" ]] || fail "License is '$license', expected '$LICENSE'"
    # %config(noreplace) shows as the flags "cn".
    config="$(rpm -q --queryformat '[%{FILEFLAGS:fflags} %{FILENAMES}\n]' prismd | grep ' /etc/prism/prismd.toml$' || true)"
    [[ "$config" == "cn /etc/prism/prismd.toml" ]] || fail "/etc/prism/prismd.toml isn't %config(noreplace): '$config'"
    # Fedora's presets leave services they don't list disabled.
    expect_service disabled inactive

    dnf remove -y prismd
    mkdir -p "$(dirname "$PRESET")"
    echo "enable $UNIT" >"$PRESET"
    dnf install -y "$package"
    # Like %systemd_post, an install enables what the presets enable but doesn't start it.
    expect_service enabled inactive

    dnf remove -y prismd
    rm -f "$PRESET"
    expect_service_removed
    ;;
  *.pkg.tar.zst)
    pacman -Syu --noconfirm
    rm -rf /run/prism
    pacman -U --noconfirm "$package"
    expect_runtime_directory
    expect_file "$UNIT_FILE"
    expect_file /usr/share/licenses/prismd/LICENSE
    expect_file /usr/share/licenses/prismd/THIRD-PARTY-LICENSES
    license="$(pacman -Qi prismd | sed -n 's/^Licenses *: //p')"
    [[ "$license" == "$LICENSE" ]] || fail "Licenses is '$license', expected '$LICENSE'"
    # The backup files run from their heading to the next blank line, one per line.
    pacman -Qii prismd | sed -n '/^Backup Files/,/^$/p' | grep -qF /etc/prism/prismd.toml \
      || fail "/etc/prism/prismd.toml isn't a backup file"
    expect_service disabled inactive

    pacman -R --noconfirm prismd
    expect_service_removed
    ;;
  *)
    echo "not a deb, rpm or pkg.tar.zst: $package" >&2
    exit 2
    ;;
esac

if ((failures > 0)); then
  exit 1
fi
echo "$package: installs, creates /run/prism and leaves $UNIT as expected"
