#!/usr/bin/env bash
# Checks a release tag names the workspace version, and that the metainfo's newest release entry,
# which app stores show, is that version.
#
# Usage: check-release-version.sh <tag>
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
METAINFO="contrib/io.github.tobytaken.Prism.metainfo.xml"

if [[ $# -ne 1 ]]; then
  echo "usage: $0 <tag>" >&2
  exit 2
fi
tag="$1"

cd "$ROOT_DIR"
version="$(bash scripts/workspace-version.sh)"
newest="$(grep -m 1 -o '<release version="[^"]*"' "$METAINFO" | cut -d '"' -f 2 || true)"
failures=0
if [[ "$tag" != "v$version" ]]; then
  echo "the tag $tag doesn't name the workspace version, v$version" >&2
  failures=1
fi
if [[ "$newest" != "$version" ]]; then
  echo "$METAINFO's newest release is '$newest'; add one for $version" >&2
  failures=1
fi
if ((failures > 0)); then
  exit 1
fi
echo "$tag: the workspace and the metainfo are at $version"
