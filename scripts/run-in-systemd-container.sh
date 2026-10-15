#!/usr/bin/env bash
# Runs a command in a throwaway Docker container of an image, booted with systemd as on a real host,
# so a package's scriptlets enable and start services as they would there. The repo is mounted at
# /src, the command's working directory.
#
# Never --privileged: a privileged container's systemd-logind handles the host's lid switch and can
# suspend the host. The container gets CAP_SYS_ADMIN and no AppArmor profile only so it can mount
# its own cgroup2 hierarchy, which systemd needs writable; /sys stays read-only and no host devices
# are passed in. logind and every sleep unit are masked before systemd boots, too.
#
# Usage: run-in-systemd-container.sh <image> <command>...
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

if [[ $# -lt 2 ]]; then
  echo "usage: $0 <image> <command>..." >&2
  exit 2
fi
image="$1"
shift

# The images leave systemd out, so this installs it, then boots it. Docker's Debian and Ubuntu
# images also ship a policy-rc.d that keeps packages from starting services, which a host doesn't.
# shellcheck disable=SC2016 # The container's shell expands this.
boot='
set -e
if command -v apt-get >/dev/null; then
  apt-get update -qq
  DEBIAN_FRONTEND=noninteractive apt-get install -qq -y systemd >/dev/null
  rm -f /usr/sbin/policy-rc.d
elif command -v dnf >/dev/null; then
  dnf install -q -y systemd >/dev/null
elif command -v pacman >/dev/null; then
  pacman -Sy --noconfirm --needed systemd >/dev/null
fi
for unit in systemd-logind.service sleep.target suspend.target hibernate.target \
  hybrid-sleep.target suspend-then-hibernate.target systemd-suspend.service \
  systemd-hibernate.service systemd-hybrid-sleep.service systemd-suspend-then-hibernate.service; do
  ln -sf /dev/null "/etc/systemd/system/$unit"
done
# Docker mounts the cgroup hierarchy read-only; a fresh mount in the private cgroup namespace is
# writable and holds only the container.
umount /sys/fs/cgroup
mount -t cgroup2 -o nosuid,nodev,noexec cgroup2 /sys/fs/cgroup
exec /usr/lib/systemd/systemd
'
container="$(docker run --detach --cgroupns=private --cap-add SYS_ADMIN \
  --security-opt apparmor=unconfined --tmpfs /run --tmpfs /run/lock --tmpfs /tmp \
  --volume "$ROOT_DIR:/src" --workdir /src "$image" sh -c "$boot")"
trap 'docker rm --force "$container" >/dev/null' EXIT

state=""
for _ in $(seq 300); do
  if [[ "$(docker inspect --format '{{.State.Running}}' "$container")" != true ]]; then
    break
  fi
  # Fails until systemd runs as PID 1; degraded means a unit the container can't run failed.
  state="$(docker exec "$container" systemctl is-system-running 2>/dev/null || true)"
  [[ "$state" == running || "$state" == degraded ]] && break
  sleep 1
done
if [[ "$state" != running && "$state" != degraded ]]; then
  echo "systemd didn't boot in $image (state '$state'):" >&2
  docker logs "$container" 2>&1 | tail -n 20 >&2
  exit 1
fi

docker exec "$container" "$@"
