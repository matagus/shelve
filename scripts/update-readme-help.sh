#!/usr/bin/env bash
# Regenerate the Usage block in README.md from the current binary's --help
# output. Keeps the single source of truth in the clap definition; the README
# is just a rendered copy that CI verifies has not drifted.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
BIN="${1:-"$ROOT/target/debug/shelve"}"

if [ ! -x "$BIN" ]; then
  echo "error: $BIN not found or not executable" >&2
  echo "usage: $0 [path-to-shelve-binary]" >&2
  exit 1
fi

HELP="$("$BIN" --help)"

README="$ROOT/README.md"

# Replace everything between the ```text fence after ## Usage and its closing
# ``` with the fresh --help output.
awk -v help="$HELP" '
  /^## Usage$/        { in_usage=1 }
  in_usage && /^```text$/ { print; printing=1; next }
  printing && /^```$/     { printf "%s\n", help; print; printing=0; in_usage=0; next }
  printing                { next }
                          { print }
' "$README" > "$README.tmp"

mv "$README.tmp" "$README"
echo "README.md updated."
