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
# Nothing here summarises or filters what the tool printed: that filtering, in
# the agent's own output-compaction layer, is what turned a clippy run exiting
# 101 into a reported "No issues found".
#
# The status is captured *inside* the branch with `|| _st=$?`. A trailing
# `return $?` after a failing `if` returns 0, because `$?` is reset by the test
# itself — so `if cmd; then ...; fi; return $?` reports success for every
# failure. Verified against dash and bash 3.2.
run_step() {
  _name=$1
  shift
  log "RUN $_name: $*"
  _st=0
  "$@" >&2 2>&1 || _st=$?
  if [ "$_st" -eq 0 ]; then
    printf '%s:ok\n' "$_name"
    return 0
  fi
  printf '%s:FAIL exit=%s\n' "$_name" "$_st"
  return "$_st"
}

# --- Clippy JSON -------------------------------------------------------------
#
# cargo --message-format=json puts one diagnostic object per line, and the fields
# that matter (level, rendered text) are nested under "message", not at top level.
# Extracting them needs a real string scanner: a rendered diagnostic contains
# `\"` and `\\n` sequences, so a `"key":"[^"]*"` pattern stops at the first
# escaped quote and silently yields a truncated or empty value. That is how an
# earlier version of this reported "0 diagnostics" for a red clippy gate.

json_field() {
  _file=$1
  _key=$2
  awk -v key="$_key" '
    {
      p = index($0, "\"" key "\":\"")
      if (p == 0) exit
      p += length(key) + 4
      out = ""
      esc = 0
      for (i = p; i <= length($0); i++) {
        c = substr($0, i, 1)
        if (esc) { out = out c; esc = 0; continue }
        if (c == "\\") { out = out c; esc = 1; continue }
        if (c == "\"") break
        out = out c
      }
      print out
      exit
    }
  ' "$_file"
}

# One `<level> <file>:<line>:<col> <lint-code> <message>` record per diagnostic,
# plus a final `@@count@@N` line. Notes and helps are dropped so the count means
# "diagnostics that can fail the build".
#
# Why this is a scanner and not a grep: rustc nests the real diagnostic under
# "message":{...}, and its FIRST "level" key belongs to a child note ("help"),
# while the top-level `"level":"error"` appears later in the same line. Reading
# either one positionally reports 0 diagnostics for a red gate — the exact false
# PASS these scripts exist to prevent. So the level comes from the rendered text
# ("error: ..." / "warning: ..."), which is unambiguous, and the code is taken
# from whichever #...::/... token it names.
clippy_diagnostics() {
  awk '
    function field(line, key,   p, out, j, c, e) {
      p = index(line, "\"" key "\":\"")
      if (p == 0) return ""
      p += length(key) + 4
      out = ""; e = 0
      for (j = p; j <= length(line); j++) {
        c = substr(line, j, 1)
        if (e) { out = out c; e = 0; continue }
        if (c == "\\") { out = out c; e = 1; continue }
        if (c == "\"") break
        out = out c
      }
      return out
    }
    # Decode the JSON string escapes rustc emitted. Order matters: the double
    # backslash goes first, so an escaped backslash followed by n is not read as a
    # newline. Every escape is matched through one alternation and replaced with
    # SUBST, then SUBST is swapped for the real character afterwards — see the
    # comment on that pair below.
    function unesc(s,   i, out) {
      gsub(/\\\\/, "", s)
      gsub(/\\n/, SUBST, s)
      gsub(/\\t/, " ", s)
      gsub(/\\"/, QUOTE, s)
      # Single-character substitution: SUBST is one character, so it can be
      # replaced positionally without matching a two-character sequence.
      out = ""
      for (i = 1; i <= length(s); i++) {
        c = substr(s, i, 1)
        if (c == SUBST) { out = out LF; continue }
        out = out c
      }
      return out
    }
    BEGIN {
      # Two ways a newline cannot be produced in this program, both hit in
      # practice: a "\n" *replacement* string reaches awk as two literal
      # characters because the shell single quotes preserve them, and an octal
      # or NR-based replacement gets re-escaped by gsub itself. So the decoded
      # newline is built once in BEGIN from sprintf("%c"), used as data, never
      # as a gsub replacement.
      LF = sprintf("%c", 012)
      SUBST = sprintf("%c", 001)   # stands in for \n
      QUOTE = sprintf("%c", 042)   # stands in for \"
    }
    /"reason":"compiler-message"/ {
      ren = unesc(field($0, "rendered"))
      if (ren == "") next
      # The render always opens with "<level>: <text>".
      if (ren !~ /^error: / && ren !~ /^warning: /) next
      lvl = ren
      sub(/:.*/, "", lvl)
      msg = ren
      sub(/^[^:]*: /, "", msg)
      nl = index(msg, LF)
      if (nl > 0) msg = substr(msg, 1, nl - 1)
      loc = ""
      if (match(ren, /--> [^ ]+/)) loc = substr(ren, RSTART + 4, RLENGTH - 4)
      # Lint name: clippy::x, or the rustc error code, from the help/note lines.
      code = ""
      if (match(ren, /clippy::[a-z_0-9-]+/))      code = substr(ren, RSTART, RLENGTH)
      else if (match(ren, /rust-[0-9]+/))         code = substr(ren, RSTART, RLENGTH)
      else if (match(ren, /\[[a-z_:]+\]$/))       code = substr(ren, RSTART + 1, RLENGTH - 2)
      n++
      printf "%s %s %s %s\n", lvl, loc, code, msg
    }
    END { printf "@@count@@%d\n", n + 0 }
  ' "$1"
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
  _td=$(cargo metadata --manifest-path "$1/Cargo.toml" --no-deps --format-version 1 2>/dev/null |
    head -c 65536 |
    tr ',' '\n' |
    awk -F '"' '/"target_directory"/ { print $4; exit }')
  [ -n "$_td" ] || die "cannot resolve cargo target directory for $1"
  # cargo reports this relative to the config file that set it
  # (`../.shared-target`), which is meaningless to a caller comparing two
  # worktrees. Anchor it to the checkout so path equality actually means
  # "these builds share an artifact cache".
  CD="$(CDPATH='' cd -- "$_td" 2>/dev/null && pwd)" || CD=""
  if [ -n "$CD" ]; then
    printf '%s\n' "$CD"
  else
    # Not created yet — resolve the parent, which does exist, and append the leaf.
    _parent=$(CDPATH='' cd -- "$_td/.." 2>/dev/null && pwd) || _parent=$_td
    printf '%s/%s\n' "$_parent" "$(basename -- "$_td")"
  fi
}

# owner/repo for a checkout, derived from its origin URL rather than hardcoded:
# the same script has to work on a fork, on CI, and after a remote is renamed.
# Handles ssh (`git@host:owner/repo.git`) and https forms; anything else returns
# empty so callers fail loudly instead of guessing.
repo_slug() {
  _url=$(git -C "$1" config --get remote.origin.url 2>/dev/null || echo '')
  [ -n "$_url" ] || return 1
  case $_url in
    *github.com:*) printf '%s\n' "${_url#*:}" | sed -e 's/\.git$//' ;;
    *github.com/*) printf '%s\n' "$_url" | sed -E -e 's#^.*github\.com/##' -e 's#\.git$##' ;;
    *) return 1 ;;
  esac
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
