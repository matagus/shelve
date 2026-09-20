#!/bin/sh
# shellcheck shell=sh
# smoke.sh — Phase 8 of /fix-gh-issue in one call: the real binary, end to end.
#
# What this covers that `cargo test` does not, and what it deliberately does NOT
# re-derive. The 21-case CLI matrix in tests/cli.rs already asserts --help,
# --version, column bounds, default/1st..5th column, two files, stdin, empty
# input, non-integer column, too-high column (+stderr, +once-only), missing file
# (+cause), malformed file, CRLF, BOM, unicode keys, wide rows, duplicate group
# keys and header-only input. Re-implementing those as shell was ~90% of phase 8
# in both measured runs, cost 4–7 turns, and got the answer wrong twice: once by
# feeding stdin through a redirect the harness read as a mismatch, once by using
# flags the fixture did not take. So the matrix is not repeated here; only the
# three things a unit test cannot reach are:
#
#   1. the RELEASE build, which CI's publish path also uses
#   2. SIGPIPE / closed stdout at the process boundary
#   3. byte-for-byte BASE vs NEW output over every fixture x column
#
# Usage:
#   scripts/smoke.sh <worktree> [base-worktree-or-checkout]
#
# With a second argument, the differential runs and SMOKE includes DIFF=ok|FAIL.
# Without it, only the absolute checks run and DIFF=skipped is reported — an
# omitted base is stated, never silently passed.
#
# Exit status: 0 only when every selected check passed.

set -eu

HERE=$(CDPATH='' cd -- "$(dirname -- "$0")" && pwd)
# shellcheck source=./lib.sh disable=SC1091
. "$HERE/lib.sh"

need_cmd git
need_cmd cargo

WT=${1:-}
BASE_WT=${2:-}
[ -n "$WT" ] || die "usage: scripts/smoke.sh <worktree> [base-worktree-or-checkout]"
[ -f "$WT/Cargo.toml" ] || die "not a cargo project: $WT"
if [ -n "$BASE_WT" ]; then
  [ -d "$BASE_WT" ] || die "base checkout does not exist: $BASE_WT"
fi

BIN=$(pkg_name "$WT")
WORK=/tmp/shelve-smoke.$$
trap 'rm -rf "$WORK" 2>/dev/null || true' EXIT INT TERM
mkdir -p "$WORK"

FAILED=''
CHECKS=0

pass() { printf '%s:ok\n' "$1"; CHECKS=$((CHECKS + 1)); }
fail() {
  CHECKS=$((CHECKS + 1))
  printf '%s:FAIL %s\n' "$1" "${2:-}"
  FAILED="$FAILED $1"
}

# --- 1. Release build --------------------------------------------------------
#
# Cold-built in a fresh worktree unless the shared target dir is warm (see
# .cargo/config.toml). This is the artifact `cargo publish` ships, so a debug-only
# pass proves less than it looks like it does.
log "SMOKE worktree=$WT bin=$BIN"
if ! run_step release_build cargo build --release --manifest-path "$WT/Cargo.toml"; then
  FAILED="$FAILED release_build"
  printf 'SMOKE=FAIL\nfailed_steps=%s\n' "$FAILED"
  exit 1
fi

TARGET_DIR=$(target_dir "$WT")
NEW_BIN="$TARGET_DIR/release/$BIN"
[ -x "$NEW_BIN" ] || die "built binary not found or not executable: $NEW_BIN"
printf 'NEW_BIN=%s\n' "$NEW_BIN"

# --- 2. Absolute behavioural checks -----------------------------------------
#
# Each is a *process-level* property: exit code and stream routing, which is what
# a shell caller sees and what assert_cmd only partly models.

# 2a. --help exits 0 and writes to stdout, not stderr.
"$NEW_BIN" --help >"$WORK/help.out" 2>"$WORK/help.err" || true
if [ -s "$WORK/help.out" ] && [ ! -s "$WORK/help.err" ]; then
  pass help_stdout
else
  fail help_stdout "stdout_bytes=$(wc -c <"$WORK/help.out" | tr -d ' ') stderr_bytes=$(wc -c <"$WORK/help.err" | tr -d ' ')"
fi

# 2b. Column 0 is rejected with a non-zero exit and a message on stderr.
if "$NEW_BIN" -c 0 "$WT/tests/inputs/tasks.csv" >"$WORK/c0.out" 2>"$WORK/c0.err"; then
  fail column_zero_rejected "exit=0 expected-nonzero"
elif [ -s "$WORK/c0.err" ] && [ ! -s "$WORK/c0.out" ]; then
  pass column_zero_rejected
else
  fail column_zero_rejected "nonzero-but-no-stderr-message"
fi

# 2c. A missing file names itself in the error. The whole point of the anyhow
#     chain is that the user learns *which* file failed, so assert the path, not
#     just the exit code.
if "$NEW_BIN" "$WORK/definitely-not-here.csv" >"$WORK/missing.out" 2>"$WORK/missing.err"; then
  fail missing_file_named "exit=0 expected-nonzero"
elif grep -q 'definitely-not-here.csv' "$WORK/missing.err"; then
  pass missing_file_named
else
  fail missing_file_named "nonzero-but-path-not-in-stderr"
fi

# 2d. Stdin mode: no filename argument, output identical to the same file passed
#     explicitly. Uses a pipe, not a `<` redirect into a loop — the mistake that
#     produced two false mismatches in a previous run, because the redirect was
#     consumed once and the second case read nothing.
_EXPLICIT=$("$NEW_BIN" -c 3 "$WT/tests/inputs/tasks.csv" 2>/dev/null || true)
_VIA_PIPE=$("$NEW_BIN" -c 3 <"$WT/tests/inputs/tasks.csv" 2>/dev/null || true)
if [ -n "$_EXPLICIT" ] && [ "$_EXPLICIT" = "$_VIA_PIPE" ]; then
  pass stdin_matches_file_arg
else
  fail stdin_matches_file_arg "explicit_bytes=$(printf '%s' "$_EXPLICIT" | wc -c | tr -d ' ') stdin_bytes=$(printf '%s' "$_VIA_PIPE" | wc -c | tr -d ' ')"
fi

# 2e. Closed stdout must exit 0, not report an error. Rust ignores SIGPIPE, so
#     the write surfaces as EPIPE; main maps that to success. Two variants: a
#     reader that stops early (`head`), and a descriptor that never existed
#     (`>&-`). Only the first can be observed from a pipeline's exit status, so
#     the second is checked via $? directly rather than through `${PIPESTATUS}`
#     (a bash array that dash and busybox ash do not support).
_head=$(command -v head || echo '')
if [ -z "$_head" ]; then
  printf 'sigpipe_closed_reader:SKIP reason=head-not-found\n'
else
  # `|| true` would mask the status we are measuring; capture it instead.
  _st=0
  { "$NEW_BIN" "$WT/tests/inputs/tasks.csv" | "$_head" -1 >/dev/null; } 2>"$WORK/pipe.err" || _st=$?
  if [ "$_st" -eq 0 ] && [ ! -s "$WORK/pipe.err" ]; then
    pass sigpipe_closed_reader
  else
    fail sigpipe_closed_reader "exit=$_st stderr=$(tr '\n' ' ' <"$WORK/pipe.err")"
  fi
fi

_st=0
"$NEW_BIN" "$WT/tests/inputs/tasks.csv" >&- 2>"$WORK/closed.err" || _st=$?
if [ "$_st" -eq 0 ] && [ ! -s "$WORK/closed.err" ]; then
  pass stdout_descriptor_closed
else
  fail stdout_descriptor_closed "exit=$_st stderr=$(tr '\n' ' ' <"$WORK/closed.err")"
fi

# 2f. Groups come out in UTF-8 *byte* order, not locale collation order —
#     documented behaviour, and the single most likely thing a change to the key
#     representation breaks. The expected sequence is computed here with sort(1)
#     forced into the C locale rather than hardcoded, so the check cannot rot
#     when a fixture gains a key; LC_ALL=C makes even that comparison byte-based,
#     which is what shelve does by storing keys as bytes.
_expect=$(LC_ALL=C "$NEW_BIN" -c 1 "$WT/tests/inputs/unicode-keys.csv" 2>/dev/null |
  sed -n 's/:$//p' | LC_ALL=C sort | tr '\n' ',')
_actual=$(LC_ALL=C "$NEW_BIN" -c 1 "$WT/tests/inputs/unicode-keys.csv" 2>/dev/null |
  sed -n 's/:$//p' | tr '\n' ',')
# Missing fixture or unparseable output is reported as SKIP, not FAIL: a refactor
# branch legitimately may predate tests/inputs/unicode-keys.csv, and failing the
# smoke gate over an absent fixture would push the agent to invent one. But it is
# never silently a PASS — without this guard a missing file yields empty stdout on
# both sides, "empty == empty" satisfies the comparison, and the vacuous result is
# indistinguishable from a real one. That is exactly what happened against a base
# predating the fixture.
if [ ! -f "$WT/tests/inputs/unicode-keys.csv" ]; then
  printf 'group_order_is_byte_order:SKIP reason=fixture-missing\n'
elif [ -z "$_actual" ]; then
  fail group_order_is_byte_order "reason=no-group-headers-parsed"
else
  # Two keys minimum: with one group there is no order to verify, and the
  # assertion would pass for any implementation that emitted a single header.
  _group_count=$(printf '%s' "$_actual" | tr ',' '\n' | grep -c . || true)
  if [ "${_group_count:-0}" -lt 2 ]; then
    fail group_order_is_byte_order "reason=fewer-than-two-groups count=$_group_count"
  elif [ "$_actual" = "$_expect" ]; then
    pass group_order_is_byte_order
    log "  group order: $_actual"
  else
    fail group_order_is_byte_order "got=[$_actual] expected-byte-order=[$_expect]"
  fi
fi

# --- 3. BASE <-> NEW differential -------------------------------------------
#
# Every fixture x every column up to the golden sweep's cap, compared byte for
# byte. This is the claim a refactor actually needs ("output did not change"),
# expressed as bytes rather than as a set of assertions someone has to trust.

DIFF_RESULT=skipped
if [ -n "$BASE_WT" ]; then
  # Three hazards this block has to handle, each of which can produce a FALSE
  # "output identical" verdict — the one outcome a differential must never give:
  #
  # (a) The shared target dir means both builds want the SAME artifact path, so
  #     building base overwrites the NEW binary that was just smoke-tested. Each
  #     side is copied into $WORK right after its own build and every comparison
  #     uses those private copies.
  #
  # (b) cargo decides what to rebuild from file mtimes, not content. A tar/cp copy
  #     preserves them, so a copied checkout looks OLDER than the cached artifact
  #     for another checkout at the same path; cargo then reuses the other
  #     commit's binary and reports success. Verified locally: a probe that
  #     changed group headers produced DIFF=ok until this was fixed.
  #
  # (c) Two checkouts of different revisions still collide in one fingerprint
  #     namespace. So each side builds into its OWN target dir, named after the
  #     exact revision it holds — CARGO_TARGET_DIR outranks .cargo/config.toml, so
  #     these two builds bypass the shared cache and cannot cross-contaminate.
  BASE_SRC_DIR="$WORK/base-src"
  mkdir -p "$BASE_SRC_DIR" || die "cannot create $BASE_SRC_DIR"
  log "copying base checkout to $BASE_SRC_DIR (excluding target/, .wt/)"
  (cd "$BASE_WT" && tar --exclude ./target --exclude ./.wt --exclude ./.shared-target -cf - .) |
    (cd "$BASE_SRC_DIR" && tar -xf -) ||
    die "could not copy base checkout to $BASE_SRC_DIR"
  find "$BASE_SRC_DIR" -type f -exec touch {} + 2>/dev/null ||
    die "could not refresh mtimes on the base copy"

  BASE_REV=$(git -C "$BASE_WT" rev-parse HEAD 2>/dev/null || echo unknown)
  NEW_REV=$(head_sha "$WT")
  [ "$BASE_REV" != unknown ] || die "cannot resolve the base revision in $BASE_WT"
  printf 'BASE_REV=%s\n' "$BASE_REV"
  printf 'NEW_REV=%s\n' "$NEW_REV"

  if [ "$BASE_REV" = "$NEW_REV" ]; then
    # Same commit on both sides makes the differential vacuous. Reported as a skip,
    # not a pass: "no difference" between a build and itself is exactly the false
    # confidence this script exists to prevent.
    printf 'DIFF=skipped reason=base-and-new-are-the-same-revision\n'
    log "note: BASE_REV == NEW_REV ($BASE_REV); nothing to compare."
    DIFF_RESULT=skipped
  else
    BASE_TARGET="$WORK/target-base-$BASE_REV"
    NEW_TARGET="$WORK/target-new-$NEW_REV"

    if run_step base_build env CARGO_TARGET_DIR="$BASE_TARGET" \
      cargo build --release --manifest-path "$BASE_SRC_DIR/Cargo.toml" &&
      [ -f "$BASE_TARGET/release/$BIN" ]; then
      cp "$BASE_TARGET/release/$BIN" "$WORK/bin-base"

      # Build NEW from the worktree into its own directory too. This recompiles
      # source that release_build already built above — deliberately. It is the
      # only way to know the bytes compared came from $WT at $NEW_REV rather than
      # from whatever cargo happened to have cached. The shared build stays warm
      # for the caller; this one buys correctness.
      if run_step new_build env CARGO_TARGET_DIR="$NEW_TARGET" \
        cargo build --release --manifest-path "$WT/Cargo.toml" &&
        [ -f "$NEW_TARGET/release/$BIN" ]; then
        cp "$NEW_TARGET/release/$BIN" "$WORK/bin-new"

        # Column cap matches tests/golden.rs (MAX_SWEPT_COLUMNS), so the two
        # sweeps agree by construction instead of drifting apart. Fixtures wider
        # than the cap are swept up to it on BOTH sides, which keeps the
        # comparison symmetric.
        COLS=5
        OUT_BASE="$WORK/out-base"
        OUT_NEW="$WORK/out-new"
        collect_outputs "$OUT_BASE" "$WORK/bin-base" "$WT/tests/inputs" "$COLS"
        collect_outputs "$OUT_NEW" "$WORK/bin-new" "$WT/tests/inputs" "$COLS"

        _diffs=0
        _compared=0
        for _o in "$OUT_NEW"/*; do
          _name=$(basename "$_o")
          _b="$OUT_BASE/$_name"
          [ -f "$_b" ] || continue
          _compared=$((_compared + 1))
          if cmp -s "$_b" "$_o"; then
            :
          else
            _diffs=$((_diffs + 1))
            printf 'diff %s=differ\n' "$_name"
          fi
        done

        if [ "$_compared" -eq 0 ]; then
          # Zero comparisons must never read as a pass: an empty fixture directory
          # or a silently failed sweep produces exactly this shape.
          fail differential "compared=0 reason=nothing-to-compare"
          DIFF_RESULT=fail
        elif [ "$_diffs" -eq 0 ]; then
          DIFF_RESULT=ok
          pass "differential(${_compared}_cases)"
        else
          fail differential "differing=$_diffs compared=$_compared"
          DIFF_RESULT=fail
        fi
      else
        FAILED="$FAILED new_build"
        DIFF_RESULT=build-failed
      fi
    else
      FAILED="$FAILED base_build"
      DIFF_RESULT=build-failed
    fi
    printf 'DIFF=%s\n' "$DIFF_RESULT"
  fi
else
  printf 'DIFF=skipped reason=no-base-checkout-given\n'
  log "note: no base checkout passed, so the BASE<->NEW differential did not run."
  log "      A refactor should compare against its merge base, e.g.:"
  log "      scripts/smoke.sh <worktree> <checkout-at-merge-base>"
fi

# --- Verdict -----------------------------------------------------------------

printf 'SMOKE_CHECKS=%s\n' "$CHECKS"
if [ -z "$FAILED" ]; then
  printf 'SMOKE=PASS\n'
  exit 0
fi
printf 'failed_steps=%s\n' "$FAILED"
printf 'SMOKE=FAIL\n'
exit 1
