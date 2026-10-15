#!/usr/bin/env bash
# Runs install-prismd-host.sh against stand-ins for cargo, sudo, install, systemd-tmpfiles and
# systemctl, and checks which systemd steps it takes, what it reports and how it exits, for a first
# install, a reinstall over a running prismd, and each step failing. Nothing on the host changes.
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
installer="$ROOT_DIR/scripts/install-prismd-host.sh"

stubs="$(mktemp -d)"
trap 'rm -rf "$stubs"' EXIT

# Each stand-in appends its command line to $FAKE_LOG. FAKE_FAIL names the step that fails:
# build, install, enable or restart. FAKE_ACTIVE=1 makes prismd.service already active.
cat >"$stubs/sudo" <<'EOF'
#!/usr/bin/env bash
exec "$@"
EOF
cat >"$stubs/cargo" <<'EOF'
#!/usr/bin/env bash
echo "cargo $*" >>"$FAKE_LOG"
[[ "${FAKE_FAIL:-}" != build ]]
EOF
cat >"$stubs/install" <<'EOF'
#!/usr/bin/env bash
echo "install $*" >>"$FAKE_LOG"
[[ "${FAKE_FAIL:-}" != install ]]
EOF
cat >"$stubs/systemd-tmpfiles" <<'EOF'
#!/usr/bin/env bash
echo "systemd-tmpfiles $*" >>"$FAKE_LOG"
EOF
cat >"$stubs/systemctl" <<'EOF'
#!/usr/bin/env bash
echo "systemctl $*" >>"$FAKE_LOG"
case "$1" in
  is-active) [[ "${FAKE_ACTIVE:-}" == 1 ]] ;;
  enable | restart) [[ "${FAKE_FAIL:-}" != "$1" ]] ;;
esac
EOF
chmod +x "$stubs"/*

failures=0
fail() {
  echo "$scenario: $1" >&2
  failures=$((failures + 1))
}

# Runs the installer for $scenario with the given FAKE_* settings, and leaves its exit status in
# $status, its output in $output and the systemctl calls that change anything in $systemd_steps.
run_installer() {
  local log="$stubs/log"
  : >"$log"
  status=0
  output="$(env PATH="$stubs:$PATH" FAKE_LOG="$log" "$@" bash "$installer" 2>&1)" || status=$?
  systemd_steps="$(grep '^systemctl ' "$log" | grep -v '^systemctl is-active ' || true)"
}

expect_status() {
  if [[ "$1" == 0 && "$status" != 0 ]]; then
    fail "exited $status, expected success. Output:"$'\n'"$output"
  elif [[ "$1" != 0 && "$status" == 0 ]]; then
    fail "succeeded, expected a failure. Output:"$'\n'"$output"
  fi
}

expect_output() {
  grep -qF -- "$1" <<<"$output" || fail "output doesn't say '$1'. Output:"$'\n'"$output"
}

expect_no_output() {
  if grep -qF -- "$1" <<<"$output"; then
    fail "output says '$1'. Output:"$'\n'"$output"
  fi
}

expect_systemd_steps() {
  [[ "$systemd_steps" == "$1" ]] \
    || fail "systemctl ran:"$'\n'"${systemd_steps:-(nothing)}"$'\n'"expected:"$'\n'"${1:-(nothing)}"
}

all_steps=$'systemctl daemon-reload\nsystemctl enable prismd\nsystemctl restart prismd'

scenario="first install"
run_installer FAKE_ACTIVE=0
expect_status 0
expect_systemd_steps "$all_steps"
expect_output "prismd is installed and started"
expect_no_output "restarted"

scenario="reinstall over a running prismd"
run_installer FAKE_ACTIVE=1
expect_status 0
expect_systemd_steps "$all_steps"
expect_output "prismd is installed and restarted"

scenario="restart fails"
run_installer FAKE_ACTIVE=1 FAKE_FAIL=restart
expect_status 1
expect_output "prismd didn't restart"
expect_output "systemctl status prismd"
expect_output "journalctl -u prismd"
expect_no_output "prismd is installed"

scenario="start fails"
run_installer FAKE_ACTIVE=0 FAKE_FAIL=restart
expect_status 1
expect_output "prismd didn't start"
expect_output "journalctl -u prismd"
expect_no_output "prismd is installed"

scenario="enable fails"
run_installer FAKE_ACTIVE=1 FAKE_FAIL=enable
expect_status 1
expect_systemd_steps $'systemctl daemon-reload\nsystemctl enable prismd'
expect_output "couldn't enable prismd.service"
expect_output "systemctl status prismd"
expect_output "journalctl -u prismd"
expect_no_output "prismd is installed"

scenario="build fails"
run_installer FAKE_ACTIVE=1 FAKE_FAIL=build
expect_status 1
expect_systemd_steps ""

scenario="file install fails"
run_installer FAKE_ACTIVE=1 FAKE_FAIL=install
expect_status 1
expect_systemd_steps ""

if ((failures > 0)); then
  exit 1
fi
echo "install-prismd-host.sh: starts, restarts and reports failures as expected"
