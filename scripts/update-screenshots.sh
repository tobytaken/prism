#!/usr/bin/env bash
# Regenerates the screenshot tests' reference PNGs in the pinned CI image, so they match what CI
# draws. With --check, runs the screenshot tests there without touching the references.
set -euo pipefail

# Keep in step with the test job in .github/workflows/ci.yml.
IMAGE="rust:1.99.0-slim-trixie@sha256:2752b332db73fdbb7dc576f06c82ed1f312005784ef913d7e04a28f5f55dc581"
PACKAGES="pkg-config libfontconfig-dev"

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

update=1
case "${1:-}" in
  "") ;;
  --check) update= ;;
  *)
    echo "usage: $0 [--check]" >&2
    exit 2
    ;;
esac

docker run --rm \
  --volume "$ROOT_DIR:/work" \
  --workdir /work \
  --volume prism-ci-cargo-registry:/usr/local/cargo/registry \
  --env CARGO_TARGET_DIR=/work/target/ci-image \
  --env PRISM_UPDATE_SCREENSHOTS="$update" \
  --env HOST_IDS="$(id -u):$(id -g)" \
  --env PACKAGES="$PACKAGES" \
  "$IMAGE" \
  bash -c '
    set -euo pipefail
    [ -n "$PRISM_UPDATE_SCREENSHOTS" ] || unset PRISM_UPDATE_SCREENSHOTS
    apt-get update -qq
    apt-get install -y -qq --no-install-recommends $PACKAGES >/dev/null
    status=0
    cargo test --locked -p prism --test screenshots || status=$?
    chown -R "$HOST_IDS" crates/prism/tests/screenshots target/ci-image
    exit $status
  '
