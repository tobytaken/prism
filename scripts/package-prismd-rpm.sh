#!/usr/bin/env bash
# Packages an already built target/release/prismd, which CI builds once on an old glibc.
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

cd "$ROOT_DIR"
cargo generate-rpm -p crates/prismd
