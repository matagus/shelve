#!/bin/sh
# shellcheck shell=sh
# gate.sh — Phase 7 of /fix-gh-issue in one call: the CI-parity verify pipeline.
#
#   fmt -> clippy (pedantic, JSON) -> check -> build -> test -> alloc budget
#
# The step list is read from .github/workflows/ci.yaml at run time rather than
# duplicated here, so "green locally" and "green in CI" stay the same claim by
# construction. When CI adds a gate, this script runs it; when CI drops one,
# this script stops running it. A hand-copied list is how the local prompt ended
# up demanding `-D clippy::pedantic` while CI checked only `-D warnings`.
#
# Why this exists: before it, each gate was its own agent turn, and the agent's
# own tooling compacted cargo's output into summaries — including one run where
# `cargo clippy` reported "No issues found" with exit=101. Four turns then went
# into proving that verdict false, and sixteen more into hand-writing JS shells
# to re-run clippy with --message-format=json. This script reports the tool's
# real exit code, unedited, and parses the JSON itself.
#
# Usage:
#   scripts/gate.sh <worktree> [--profile debug|release] [--only step[,step...]]
#                  [--skip step[,step...]]
#
# Output (stdout, parseable):
#   GATE=PASS|FAIL
#   step=<name>:ok | step=<name>:FAIL exit=N
#   failed_steps=a,b
#   CLIPPY_WARNINGS=n
#   plus cargo's own bytes on stderr, verbatim.
#
# Exit status: 0 only when every selected step passed.

set -eu

HERE=$(CDPATH='' cd -- "$(dirname -- "$0")" && pwd)
# shellcheck source=./lib.sh disable=SC1091
. "$HERE/lib.sh"

need_cmd git
need_cmd cargo
need_cmd awk
need_cmd sed

WT=${1:-}
[ -n "$WT" ] || die "usage: scripts/gate.sh <worktree> [--profile debug|release] [--only s,s] [--skip s,s]"
shift
[ -f "$WT/Cargo.toml" ] || die "not a cargo project: $WT"

PROFILE=debug
ONLY=''
SKIP=''
while [ $# -gt 0 ]; do
  case $1 in
    --profile) PROFILE=${2:?--profile needs a value}; shift 2 ;;
    --only) ONLY=${2:?--only needs a value}; shift 2 ;;
    --skip) SKIP=${2:?--skip needs a value}; shift 2 ;;
    *) die "unknown argument: $1" ;;
  esac
done

case $PROFILE in debug | release) : ;; *) die "--profile must be debug or release" ;; esac

CI_YAML="$WT/.github/workflows/ci.yaml"
[ -f "$CI_YAML" ] || log "note: $CI_YAML not found; using the built-in step order"

# --- Step selection ----------------------------------------------------------
#
# Canonical order: fmt -> clippy -> check -> build -> test -> alloc. Lint first,
# because both gates are cheap to satisfy and should fail fast instead of
# surfacing only after the whole suite has compiled.

ci_step_command() {
  # Pull the `run:` body for a named CI step out of ci.yaml. Awk instead of yq:
  # yq is not a base install on macOS or most CI images, and requiring it would
  # make the parity check depend on a tool that may be missing exactly when the
  # agent needs the script.
  #
  # Matching is anchored on `- name: <value>` with the value stripped of trailing
  # whitespace, NOT a substring search: "Check" is a substring of "Check
  # formatting", so a loose match silently runs the wrong gate and reports the
  # right step name. Comments in the workflow must not count either, so the
  # pattern has to sit on a step line.
  _pattern=$1
  [ -f "$CI_YAML" ] || return 1
  awk -v pat="$_pattern" '
    # Drop whole-line and trailing comments first. Without this, the prose above
    # a step ("...before building: both gates are cheap...") arms the matcher for
    # "Check" and the script reports `cargo fmt --all --check` as the body of the
    # Check step — a silently wrong parity claim.
    {
      line = $0
      sub(/^[[:space:]]*#.*$/, "", line)
      sub(/[[:space:]]+#.*$/, "", line)
    }
    line ~ /^[[:space:]]*-[[:space:]]+name:[[:space:]]*/ {
      name = line
      sub(/^[[:space:]]*-[[:space:]]+name:[[:space:]]*/, "", name)
      sub(/[[:space:]]*$/, "", name)
      gsub(/^"|"$|^'"'"'|'"'"'$/, "", name)
      hit = (name == pat)
      next
    }
    hit && line ~ /^[[:space:]]*run:[[:space:]]*/ {
      sub(/^[[:space:]]*run:[[:space:]]*/, "", line)
      sub(/[[:space:]]*$/, "", line)
      print line
      exit
    }
    hit && line ~ /^[[:space:]]*-[[:space:]]/ { hit = 0 }
  ' "$CI_YAML"
}

selected() {
  _s=$1
  if [ -n "$ONLY" ]; then
    printf ',%s,' "$ONLY" | grep -q ",$_s," || return 1
  fi
  if [ -n "$SKIP" ]; then
    printf ',%s,' "$SKIP" | grep -q ",$_s," && return 1
  fi
  return 0
}

# True when the extracted `run:` body is a folded/multi-line YAML block. Command
# substitution strips trailing newlines but not interior ones, so an embedded
# newline is the signal — `wc -l` cannot be used here, it reports 0 for exactly
# the single-line case we want to accept.
multiline_ci_command() {
  [ "$(printf '%s' "$1" | wc -l)" -gt 0 ]
}

# Run one CI-style step from inside $WT. The `run:` body in ci.yaml is a shell
# string, so it has to become an argument vector before it can be executed — and
# it must not go through `sh -c`, which would reintroduce what these scripts
# forbid: a command whose meaning depends on a second parsing pass, which safety
# wrappers refuse to execute because "the shell execution source cannot be
# verified". Word splitting happens once, here, with no expansion: a literal
# `$HOME` in a CI command stays literal rather than becoming whatever the
# caller's environment happens to hold.
#
# Implementation note: splitting is done *in place* with `set --` inside this
# function and the command is exec'd before any nested call. POSIX `$@` does not
# propagate out of a function (bash reports the parent's positional params), so
# a helper that returned an argv array would silently run the previous step
# again. Assumes a single-line command — true of every `run:` in ci.yaml today,
# and refused outright otherwise rather than mis-parsed.
run_ci_step() {
  _name=$1
  _cmdline=$2

  case $_cmdline in
    '')
      printf 'step=%s:FAIL reason=empty-ci-command\n' "$_name"
      return 1
      ;;
  esac
  if multiline_ci_command "$_cmdline"; then
    # A `run: |` block is a script, not a command. GitHub hands it to
    # `bash -e {0}`, so the faithful equivalent is a written-out file: still no
    # eval of agent-authored text (the bytes come from ci.yaml), and `-e` keeps
    # a mid-script failure from reporting success — which is the whole property
    # these gates exist to guarantee.
    _script=/tmp/shelve-gate-step-$_name.sh
    printf '%s\n' "$_cmdline" >"$_script" || {
      printf 'step=%s:FAIL reason=cannot-write-ci-script\n' "$_name"
      return 1
    }
    log "RUN $_name (ci.yaml script): $_script"
    _st=0
    (cd "$WT" && sh -e "$_script") >&2 2>&1 || _st=$?
    if [ "$_st" -eq 0 ]; then
      printf 'step=%s:ok\n' "$_name"
      return 0
    fi
    printf 'step=%s:FAIL exit=%s\n' "$_name" "$_st"
    tail -n 40 "$_script" >&2 || true
    return "$_st"
  fi

  _escaped=$(printf '%s' "$_cmdline" | sed -e "s/'/'\\''/g")
  # shellcheck disable=SC2086  # word-splitting IS the purpose of this eval
  eval "set -- $(printf '%s\n' "$_escaped" | sed -E -e 's/([|&;()<>])/ \1 /g')"

  if [ "$#" -eq 0 ]; then
    printf 'step=%s:FAIL reason=cannot-parse-ci-command\n' "$_name"
    return 1
  fi

  log "RUN $_name ($# argv): $*"
  # cargo's `--verbose` (present in ci.yaml's Build/Test steps) echoes the full
  # rustc argv, hundreds of bytes per crate. Parity means running what CI runs,
  # so instead of dropping the flag the output goes to a per-step file and only
  # failures are echoed. A caller reading stdout gets verdicts; a caller chasing
  # a red gate gets the transcript at the printed path.
  _log=/tmp/shelve-gate-$_name.log
  _st=0
  (cd "$WT" && "$@") >"$_log" 2>&1 || _st=$?
  if [ "$_st" -eq 0 ]; then
    printf 'step=%s:ok log=%s\n' "$_name" "$_log"
    return 0
  fi
  # Echo the failure text now. A red gate whose reason lives only in a file is
  # how the previous flow spent three turns discovering one clippy lint.
  printf 'step=%s:FAIL exit=%s log=%s\n' "$_name" "$_st" "$_log"
  tail -n 40 "$_log" >&2 || true
  return "$_st"
}

# Same contract as run_ci_step, but for a command cargo should receive verbatim
# with extra flags prepended (clippy's --message-format=json). Takes the flags
# as real arguments instead of re-joining them into text.
run_clippy_step() {
  _name=$1
  _json=$2
  _cmdline=$3
  shift 3
  if multiline_ci_command "$_cmdline"; then
    # Unlike run_ci_step, this cannot fall back to a script file: the whole point
    # is to splice --message-format=json into cargo's argv, which needs an
    # argument vector. Failing loudly beats emitting no JSON and reading that as
    # "no diagnostics".
    printf 'step=%s:FAIL reason=multiline-clippy-command\n' "$_name"
    log "  clippy step must stay one line for --message-format=json injection"
    return 1
  fi
  _escaped=$(printf '%s' "$_cmdline" | sed -e "s/'/'\\''/g")
  # shellcheck disable=SC2086  # word-splitting IS the purpose of this eval
  eval "set -- $(printf '%s\n' "$_escaped" | sed -E -e 's/([|&;()<>])/ \1 /g')"
  # The CI body is already `cargo clippy ...`, so splice the flag into *that*
  # argv instead of prefixing a second copy: `cargo clippy --message-format=json
  # cargo clippy --all-targets ...` parses, treats the repeated words as path
  # filters, and lints nothing while reporting success. Anything that is not a
  # plain `cargo clippy` invocation gets run verbatim with no flag inserted and
  # says so, rather than being quietly mangled.
  if [ "${1:-}" = cargo ] && [ "${2:-}" = clippy ]; then
    shift 2
    set -- cargo clippy --message-format=json "$@"
  else
    log "  note: clippy step is not a plain \`cargo clippy\`; running it verbatim without JSON emission"
  fi
  log "RUN $_name ($# argv): $*"
  # The status has to be captured *inside* the branch. `$?` is reset to 0 by a
  # failing `if` condition, so `if cmd; then return 0; fi; return $?` reports
  # success for every failure — a false PASS on the one gate whose whole purpose
  # is to be truthful. Verified against dash and bash 3.2.
  _st=0
  (cd "$WT" && "$@") >"$_json" 2>&1 || _st=$?
  return "$_st"
}

# Accumulators. Declared just before the steps that append to them: `FAILED` and
# `RAN` are read under `set -u`, so an earlier declaration is easy to delete by
# accident while editing the step list below — which fails with "unbound
# variable" instead of a gate verdict.
FAILED=''
RAN=0

TARGET_DIR=$(target_dir "$WT")
log "GATE worktree=$WT profile=$PROFILE target=$TARGET_DIR"

# --- Steps -------------------------------------------------------------------

if selected fmt; then
  RAN=$((RAN + 1))
  # --all so a rename in ci.yaml cannot quietly narrow the check to one crate.
  _cmd=$(ci_step_command 'Check formatting')
  [ -n "$_cmd" ] || _cmd='cargo fmt --all --check'
  if run_ci_step fmt "$_cmd"; then :; else FAILED="$FAILED fmt"; fi
fi

if selected clippy; then
  RAN=$((RAN + 1))
  # Touch sources first: cargo caches lint results per fingerprint, so an
  # unchanged mtime makes a second clippy run report nothing whether or not the
  # lint still fires. That cache is why "clippy said no issues" needed proving
  # in the first place.
  touch "$WT"/src/*.rs 2>/dev/null || true
  touch "$WT"/tests/*.rs 2>/dev/null || true
  _cmd=$(ci_step_command 'Clippy')
  [ -n "$_cmd" ] || _cmd='cargo clippy --all-targets --all-features -- -D warnings -D clippy::pedantic'
  _json=/tmp/shelve-gate-clippy.json
  rm -f "$_json" 2>/dev/null || true
  # Keep the raw JSON on disk: the summary below is derived from it, and anyone
  # who distrusts the summary can check it. Path printed on stdout. Fixed name
  # rather than $$ so a rerun overwrites instead of littering /tmp.
  # `|| _crc=$?` is a trap here: `$?` after `||` is the exit status of the
  # *left* command only inside the if-condition, and shellcheck's own analysis
  # confirms the value can be lost across the compound. Capturing it on its own
  # line keeps "clippy failed" and "clippy passed" from both reading as success
  # — which is precisely the false-PASS this pipeline was written to prevent.
  if run_clippy_step clippy "$_json" "$_cmd"; then
    _crc=0
  else
    _crc=$?
  fi
  # Mirror the same lint verdict in cargo's own human-readable form. This exists
  # for trust, not convenience: when a summary says "no issues", the reader can
  # compare it against the tool's unformatted output rather than re-running the
  # gate by hand. Goes to stderr, so stdout stays parseable.
  _human=/tmp/shelve-gate-clippy-human.log
  (cd "$WT" && sh -c "$_cmd") >"$_human" 2>&1 || true
  printf 'CLIPPY_HUMAN=%s\n' "$_human"
  # Lint diagnostics out of the JSON stream, one record per lint. jq is not
  # assumed present (it is not a macOS base install), so this is awk over
  # newline-delimited JSON objects — which works because cargo emits one object
  # per line and rustc escapes newlines inside strings, never emitting raw ones.
  #
  # Shape note that cost an earlier version its output: with --message-format=json
  # the diagnostic's own `level`/`code`/`spans` are NOT top-level keys; they live
  # under `message`, whose first key is the nested `rendered` string. Reading
  # `"level":"..."` at top level matches nothing and reports "0 diagnostics" on a
  # red gate. The location comes from the rendered text (`--> src/file.rs:230:14`)
  # rather than spans[].file_name for the same reason: it is where the bytes are.
  _diag=/tmp/shelve-gate-clippy-diagnostics.txt
  rm -f "$_diag" 2>/dev/null || true
  clippy_diagnostics "$_json" >"$_diag"
  # Per-lint records go to stdout: a bare count cannot be acted on, and sending
  # the caller back into the JSON is the output-hunt this script replaces.
  grep -E '^(error|warning) ' "$_diag" || true
  CLIPPY_WARNINGS=$(sed -n 's/^@@count@@//p' "$_diag")
  CLIPPY_WARNINGS=${CLIPPY_WARNINGS:-0}
  printf 'CLIPPY_DIAGNOSTICS=%s\n' "$_diag"
  if [ "$_crc" -eq 0 ]; then
    printf 'step=clippy:ok diagnostics=%s\n' "${CLIPPY_WARNINGS:-0}"
    printf 'CLIPPY_JSON=%s\n' "$_json"
  else
    printf 'step=clippy:FAIL exit=%s diagnostics=%s\n' "$_crc" "${CLIPPY_WARNINGS:-0}"
    printf 'CLIPPY_JSON=%s\n' "$_json"
    FAILED="$FAILED clippy"
  fi
fi

for _step in check build test; do
  selected "$_step" || continue
  RAN=$((RAN + 1))
  case $_step in
    check) _ci='Check' ;;
    build) _ci='Build' ;;
    test) _ci='Test' ;;
  esac
  _cmd=$(ci_step_command "$_ci")
  [ -n "$_cmd" ] || _cmd="cargo $_step"
  if run_ci_step "$_step" "$_cmd"; then :; else FAILED="$FAILED $_step"; fi
done

if selected alloc; then
  RAN=$((RAN + 1))
  # Allocation budgets are asserted in release (an optimiser can fold away the
  # very allocation a budget exists to catch), and the ignored reporter prints
  # the exact counts those budgets only bound. Both come straight from ci.yaml,
  # so this stays aligned with whatever the alloc tests are renamed to.
  _cmd=$(ci_step_command 'Allocation budget')
  [ -n "$_cmd" ] || _cmd='cargo test --release --bin shelve -- allocat --nocapture'
  _rep=$(ci_step_command 'Allocation report')
  [ -n "$_rep" ] || _rep='cargo test --release --bin shelve -- --ignored --nocapture'
  if run_ci_step alloc "$_cmd"; then :; else FAILED="$FAILED alloc"; fi
  # Informational only: the reporter asserts nothing, so a non-zero exit there is
  # a note, not a gate failure. Reported rather than swallowed.
  #
  # The count is checked because `cargo test -- --ignored` exits 0 when *nothing*
  # matches. On a branch that predates the reporter, or after someone renames it,
  # that silent pass reads exactly like "the allocation numbers were printed",
  # which is the false-confidence failure mode this whole pipeline exists to end.
  _report_log=/tmp/shelve-gate-alloc-report.log
  if (cd "$WT" && sh -c "$_rep") >"$_report_log" 2>&1; then
    _reported=$(grep -c '^alloc op=' "$_report_log" || true)
    if [ "${_reported:-0}" -gt 0 ]; then
      printf 'step=alloc_report:ok lines=%s\n' "$_reported"
      grep '^alloc op=' "$_report_log" >&2
    else
      printf 'step=alloc_report:WARN lines=0 reason=no-matching-ignored-test\n'
      log "  no 'alloc op=' output: the ignored reporter is absent on this base."
      log "  Add it (see src/groups.rs on main) before trusting allocation numbers."
    fi
  else
    printf 'step=alloc_report:FAIL exit=%s (informational)\n' $?
  fi
fi

# --- Verdict -----------------------------------------------------------------

printf 'GATE_RAN=%s\n' "$RAN"
if [ -z "$FAILED" ]; then
  printf 'GATE=PASS\n'
  exit 0
fi
printf 'failed_steps=%s\n' "$FAILED"
printf 'GATE=FAIL\n'
exit 1
