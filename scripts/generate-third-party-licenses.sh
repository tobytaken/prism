#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

cd "$ROOT_DIR"
cargo about generate --fail --locked --workspace --output-file THIRD-PARTY-LICENSES third-party-licenses.hbs
