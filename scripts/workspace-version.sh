#!/usr/bin/env bash
# Prints the workspace version from Cargo.toml at a git ref, or in the checkout without one.
#
# Usage: workspace-version.sh [<git ref>]
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

cd "$ROOT_DIR"
if [[ $# -gt 0 ]]; then
  manifest="$(git show "$1:Cargo.toml")"
else
  manifest="$(cat Cargo.toml)"
fi
version="$(sed -n 's/^version = "\(.*\)"$/\1/p' <<<"$manifest")"
if [[ -z "$version" ]]; then
  echo "Cargo.toml${1:+ at $1} has no workspace version" >&2
  exit 1
fi
echo "$version"
