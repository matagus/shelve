# shellcheck shell=sh
# Shared helpers for the scripts/{preflight,gate,smoke,ship}.sh family. Not
# executable on its own: every entrypoint sources it with
#   . "$(dirname "$0")/lib.sh"
#
# Contract every script in this directory obeys:
#
#   POSIX sh only. No bashisms (no arrays, no `${a//b/c}`, no `$'...'`, no
#   `[[`), no `eval`, no GNU-only flags (`cat -A`, `date -d`, `readlink -f`,
#   `timeout`). Every script must behave identically under `/bin/sh` (bash 3.2
#   on macOS), dash, and busybox ash, and must survive `set -eu`. The callers
#   are coding agents whose tooling wraps and rewrites shell commands, so
#   anything that only works in an interactive login shell produces output the
#   caller cannot trust — which is the failure mode these scripts exist to end.
#
#   Machine-readable stdout. Each script emits `KEY=value` assignments plus
#   `<step>:ok` / `<step>:FAIL exit=N` markers, one per line. Agents parse
#   those; humans read them. Prose diagnostics go to stderr via `log`, so
#   stdout stays parseable even when a step is chatty.
#
#   Truthful verdicts. A gate that failed exits non-zero and says so on a
#   `GATE=`/`SMOKE=`/`SHIP=` line. Nothing here summarises, filters, or
#   aggregates a tool's verdict: the tool's own bytes reach stderr unmodified.

# Directory holding this script, and the repository root the script was invoked
# from. Entrypoints override both: SCRIPT_DIR because they know their own path,
# REPO_ROOT because preflight runs in main while gate/smoke run in a worktree.
# The `:-` guards matter under a caller's `set -u`: an unset $0 or $PWD would
# otherwise abort inside these assignments, before any diagnostic could print.
# shellcheck disable=SC2034  # read by entrypoints that source this file
SCRIPT_DIR=$(CDPATH='' cd -- "$(dirname -- "${0:-.}")" && pwd)
REPO_ROOT=$(CDPATH='' cd -- "${PWD:-.}" && pwd)

# shellcheck disable=SC2329  # invoked by every entrypoint that sources this file
log() { printf '%s\n' "$*" >&2; }

die() {
  printf 'FATAL %s\n' "$*" >&2
  exit 1
}

need_cmd() {
  command -v "$1" >/dev/null 2>&1 || die "required command not found: $1"
}

# Integer arithmetic without bash `$(( ))`, so callers can build loop counters
# and column ranges portably.
math() { awk "BEGIN { printf \"%d\", ($*) }"; }

# Run a command with its real stdout+stderr streamed to our stderr, then report
# the tool's own exit code verbatim. `$1` labels the step in the result line.
run_step() {
  _name=$1
  shift
  log "RUN $_name: $*"
  if "$@" >&2 2>&1; then
    printf '%s:ok\n' "$_name"
    return 0
  fi
  _rc=$?
  printf '%s:FAIL exit=%s\n' "$_name" "$_rc"
  return "$_rc"
}

# --- Cargo project facts -----------------------------------------------------

# Package name from the manifest. Parsed with awk rather than
# `cargo metadata --jq` (needs jq) or python (not guaranteed present).
pkg_name() {
  awk '
    /^\[package\]/ { inpkg = 1; next }
    /^\[/          { inpkg = 0 }
    inpkg && /^name[ \t]*=/ { sub(/^[^=]*=[ \t]*/, ""); gsub(/^"|"$/, ""); print; exit }
  ' "$1/Cargo.toml"
}

# Explicit [[bin]] targets, one per line (handles both inline and block form).
pkg_bins() {
  awk '
    /^\[\[bin\]\]/ { inbin = 1; next }
    /^\[/          { inbin = 0 }
    inbin && /^name[ \t]*=/ { sub(/^[^=]*=[ \t]*/, ""); gsub(/^"|"$/, ""); print }
  ' "$1/Cargo.toml"
}

# Bins to smoke: declared [[bin]] entries if any, else cargo's default of one
# bin named after the package.
project_bins() {
  _bins=$(pkg_bins "$1")
  if [ -n "$_bins" ]; then
    printf '%s\n' "$_bins"
  else
    pkg_name "$1"
  fi
}

# Absolute target directory actually in force for the project at `$1`. Reported
# rather than assumed, because it comes from .cargo/config.toml + CARGO_TARGET_DIR
# and the whole point of sharing it across worktrees is to know it worked.
# Absolute target directory actually in force for the project at `$1`. Reported
# rather than assumed: it comes from .cargo/config.toml and/or CARGO_TARGET_DIR,
# and the point of sharing one across worktrees is to be able to prove it worked.
#
# `target_directory` is the first key of that name in cargo's metadata, so `head`
# both bounds the stream and lets the pipeline finish early.
target_dir() {
  cargo metadata --manifest-path "$1/Cargo.toml" --no-deps --format-version 1 2>/dev/null |
    head -c 65536 |
    tr ',' '\n' |
    awk -F '"' '/"target_directory"/ { print $4; exit }'
}

# --- Git helpers -------------------------------------------------------------

head_sha() {
  git -C "$1" rev-parse HEAD 2>/dev/null || die "cannot resolve HEAD in $1"
}

short_sha() {
  git -C "$1" rev-parse --short=7 HEAD 2>/dev/null || die "cannot resolve HEAD in $1"
}

current_branch() {
  git -C "$1" symbolic-ref --quiet --short HEAD 2>/dev/null || short_sha "$1"
}

# Fails if the checkout has staged, unstaged, or untracked changes. Untracked
# counts: scratch left behind would otherwise be mistaken for part of the change
# under review, and `git add <explicit paths>` in ship.sh would never catch it.
require_clean_tree() {
  _st=$(git -C "$1" status --porcelain --untracked-files=all)
  if [ -n "$_st" ]; then
    log "working tree is not clean ($1):"
    printf '%s\n' "$_st" >&2
    die "commit or remove the above before continuing"
  fi
}

# --- Fixture sweep -----------------------------------------------------------
#
# Runs `$bin` over every fixture x column pair into $outdir. Deliberately does
# NOT fail the script when the binary exits non-zero: for error fixtures
# (missing/malformed file, out-of-range column) a non-zero exit *is* the
# behaviour under test, and both sides of a differential get identical
# treatment. Column count is capped by the caller against each fixture's width.
collect_outputs() {
  _outdir=$1
  _bin=$2
  _fixtures=$3
  _cols=$4
  mkdir -p "$_outdir" || return 1
  for _f in "$_fixtures"/*.csv; do
    [ -f "$_f" ] || continue
    _base=$(basename "$_f" .csv)
    _c=1
    while [ "$_c" -le "$_cols" ]; do
      "$_bin" -c "$_c" "$_f" >"$_outdir/${_base}-c${_c}.out" 2>"$_outdir/${_base}-c${_c}.err" || true
      _c=$(math "$_c + 1")
    done
  done
}

# Number of columns in a CSV's header row, by counting top-level commas. Good
# enough for fixtures (none quote a comma in their header); tests/golden.rs does
# the same thing properly with csv::ReaderBuilder.
csv_header_columns() {
  head -n 1 "$1" | tr -cd ',' | awk 'BEGIN { n = 0 } { n += length($0) } END { print n + 1 }'
}
