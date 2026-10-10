#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
FONTS_DIR="crates/prism/ui/fonts"

cd "$ROOT_DIR"
cargo about generate --fail --locked --workspace --output-file THIRD-PARTY-LICENSES third-party-licenses.hbs

# cargo-about only knows crates; the GUI also embeds these fonts.
{
  printf '\n%s\n' "--------------------------------------------------------------------------------"
  printf '%s\n\n' "SIL Open Font License 1.1 (OFL-1.1)"
  printf '%s\n' "Used by:"
  printf '%s\n' "- Inter, embedded in prism (https://github.com/rsms/inter)"
  printf '%s\n' "- JetBrains Mono, embedded in prism (https://github.com/JetBrains/JetBrainsMono)"
  for license in "$FONTS_DIR/Inter-OFL.txt" "$FONTS_DIR/JetBrainsMono-OFL.txt"; do
    printf '\n'
    cat "$license"
  done
} >> THIRD-PARTY-LICENSES
