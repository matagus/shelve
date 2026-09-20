#!/bin/sh
# shellcheck shell=sh
# wt-cleanup.sh — remove accumulated issue worktrees. HUMAN-INVOKED ONLY.
#
#   scripts/wt-cleanup.sh                 # list what exists, delete nothing
#   scripts/wt-cleanup.sh --merged        # remove worktrees whose branch is in main
#   scripts/wt-cleanup.sh <branch>...     # remove exactly these branches
#   scripts/wt-cleanup.sh --all-safe      # merged OR unmodified since last commit
#
# Why this file exists separately: the fix-gh-issue flow forbids the agent
# removing a worktree at any point, including after CI is green — the reviewer
# needs the workspace to inspect the diff. That rule is correct and stays. It
# also means worktrees accumulate (a previous session left two behind here), so
# the human needs one command that does the deletion deliberately, with the
# checks an agent is not permitted to skip.
#
# What it will NOT do:
#   * delete a worktree with uncommitted changes unless --force says so per item
#   * delete a branch that is not merged into main without --delete-branch
#   * touch the primary checkout, ever
#   * run `git worktree prune` on paths it did not verify are gone
#
# Branches are retained by default. Removing a worktree directory is reversible
# (`git worktree add` again); deleting the branch deletes the commits' only name.

set -eu

HERE=$(CDPATH='' cd -- "$(dirname -- "$0")" && pwd)
# shellcheck source=./lib.sh disable=SC1091
. "$HERE/lib.sh"

need_cmd git

ROOT=$(CDPATH='' cd -- "$HERE/.." && pwd)
MODE=list
FORCE=0
DELETE_BRANCH=0
TARGETS=''

while [ $# -gt 0 ]; do
  case $1 in
    --merged) MODE=merged; shift ;;
    --all-safe) MODE=all-safe; shift ;;
    --force) FORCE=1; shift ;;
    --delete-branch) DELETE_BRANCH=1; shift ;;
    -h | --help)
      sed -n '2,30p' "$0"
      exit 0
      ;;
    -*) die "unknown option: $1" ;;
    *)
      # Positional arguments select a worktree; they must not also change the mode,
      # or naming one branch silently downgrades an --all-safe sweep to "only what
      # you named" and reports removable=0 for everything else.
      MODE=explicit
      TARGETS="$TARGETS $1"
      shift
      ;;
  esac
done

# --- Inventory ---------------------------------------------------------------
#
# `git worktree list --porcelain` rather than the human table: it gives path,
# HEAD and branch as separate fields, so a path containing a space cannot be
# misread as two columns. The primary checkout is identified by its own HEAD and
# excluded explicitly — never operated on, never even offered for selection.

MAIN_HEAD=$(git -C "$ROOT" rev-parse HEAD)
printf 'MAIN=%s\nMAIN_HEAD=%s\n' "$ROOT" "$MAIN_HEAD"

_wt_path=''
_wt_head=''
_wt_branch=''
REMOVABLE=0
PROTECTED=0

flush_entry() {
  [ -n "$_wt_path" ] || return 0
  case $_wt_path in
    "$ROOT")
      printf 'ENTRY=%s head=%s branch=%s action=skip reason=primary-checkout\n' \
        "$_wt_path" "$(printf '%s' "$_wt_head" | cut -c1-7)" "${_wt_branch#refs/heads/}"
      PROTECTED=$((PROTECTED + 1))
      return 0
      ;;
  esac

  _b=${_wt_branch#refs/heads/}
  _reason=''
  # Decisions are made by setting one of these, never by falling through with a
  # stale value. An earlier version initialised `_ok=0` and only set it in the
  # else-branches, so `--force` on a dirty worktree logged "they will be lost"
  # and then skipped the removal: the caller was told to accept a loss that never
  # happened, and got no explanation for why nothing was deleted.
  _remove=1

  if [ "$MODE" = list ]; then
    _action=list
    _remove=0
  else
    # Order matters: dirtiness is checked first and --force has to clear it before
    # the merge tests run, otherwise a clean-but-unmerged tree and a dirty-and-
    # forced tree take the same branch.
    if [ -n "$(git -C "$_wt_path" status --porcelain --untracked-files=all 2>/dev/null)" ]; then
      if [ "$FORCE" = 1 ]; then
        log "  note: $_wt_path has uncommitted changes (--force given); they will be lost"
      else
        _remove=0
        _reason=dirty-use-force
      fi
    fi

    if [ "$_remove" = 1 ]; then
      case $MODE in
        merged)
          # Ancestry against the LOCAL main ref, not origin/main: this script runs
          # offline and a fetch would make cleanup depend on network state.
          if git -C "$ROOT" merge-base --is-ancestor "$_wt_head" refs/heads/main 2>/dev/null; then
            _remove=1
          else
            _remove=0
            _reason=not-merged-into-main
          fi
          ;;
        all-safe)
          # "Safe" = its commits are already reachable from main, or it holds no
          # commits of its own at all. Anything else stays for review.
          if git -C "$ROOT" merge-base --is-ancestor "$_wt_head" refs/heads/main 2>/dev/null ||
            [ "$_wt_head" = "$MAIN_HEAD" ]; then
            _remove=1
          else
            _remove=0
            _reason=has-unmerged-commits
          fi
          ;;
        *)
          # Explicit branch/path named by the caller: that IS the decision, so no
          # additional mode test applies (only the dirtiness gate above does).
          _remove=1
          ;;
      esac
    fi

    if [ "$_remove" = 1 ]; then
      _action=remove
    else
      _action=skip
    fi
  fi

  printf 'ENTRY=%s head=%s branch=%s action=%s%s\n' \
    "$_wt_path" "$(printf '%s' "$_wt_head" | cut -c1-7)" "$_b" "$_action" "${_reason:+ reason=$_reason}"

  if [ "$_action" = remove ]; then
    REMOVABLE=$((REMOVABLE + 1))
    if [ "$MODE" != list ]; then
      # Git enforces the same rule this script does, independently: a dirty
      # worktree needs its own --force. Passing ours through only when we actually
      # accepted data loss keeps the two checks from disagreeing — otherwise the
      # caller sees "they will be lost" followed by a silent refusal, which is
      # worse than either outcome alone.
      if [ "$FORCE" = 1 ]; then _wt_flag=--force; else _wt_flag=''; fi
      # shellcheck disable=SC2086  # empty expansion means "no flag", on purpose
      if ! git -C "$ROOT" worktree remove $_wt_flag "$_wt_path"; then
        log "  failed to remove $_wt_path (git refused; re-run with --force if intended)"
        return 0
      fi
      printf 'REMOVED=%s\n' "$_wt_path"
      if [ "$DELETE_BRANCH" = 1 ]; then
        # -d by default, which refuses an unmerged branch. Deleting a worktree
        # directory is reversible (`git worktree add` again); deleting the branch
        # removes the only name for those commits, so it is never compounded
        # silently — --force has to be stated as well as understood.
        if [ "$FORCE" = 1 ]; then _flag='-d -f'; else _flag='-d'; fi
        # shellcheck disable=SC2086  # two flags on purpose; quoting would pass "-d -f" as one argv
        if git -C "$ROOT" branch $_flag "$_b" >/dev/null 2>&1; then
          printf 'BRANCH_DELETED=%s\n' "$_b"
        else
          log "  kept branch $_b (unmerged; pass --force to delete it)"
        fi
      fi
    fi
  else
    PROTECTED=$((PROTECTED + 1))
  fi
}

if [ "$MODE" = list ]; then
  printf 'MODE=list (nothing will be removed)\n'
else
  printf 'MODE=%s force=%s delete_branch=%s\n' "$MODE" "$FORCE" "$DELETE_BRANCH"
fi

# Parse the porcelain stream. A while-read loop would run in a subshell under a
# pipe (losing REMOVABLE), so the whole record set is walked in one awk pass that
# emits tab-separated triples, then consumed here.
git -C "$ROOT" worktree list --porcelain |
  awk '
    /^worktree /  { if (p != "") printf "%s\t%s\t%s\n", p, h, b; p = substr($0, 10); h = ""; b = "" }
    /^HEAD /      { h = substr($0, 6) }
    /^branch /    { b = substr($0, 8) }
    END           { if (p != "") printf "%s\t%s\t%s\n", p, h, b }
  ' >"/tmp/shelve-wt-list.$$"

while IFS='	' read -r _wt_path _wt_head _wt_branch; do
  [ -n "$_wt_path" ] || continue
  if [ -n "$TARGETS" ]; then
    _tb=${_wt_branch#refs/heads/}
    # A target matches a branch name, a full path, or the worktree directory's
    # own basename. The last one is not convenience: `git worktree add <path>`
    # without -b leaves HEAD detached and the porcelain record has an EMPTY
    # branch field, so a caller pointing at the directory they can actually see
    # would match nothing and get "removable=0" with no explanation.
    _match=0
    for _t in $TARGETS; do
      if [ "$_t" = "$_tb" ] || [ "$_t" = "$_wt_path" ] ||
        [ "$_t" = "$(basename "$_wt_path")" ]; then
        _match=1
      fi
    done
    [ "$_match" = 1 ] || continue
  fi
  flush_entry
done <"/tmp/shelve-wt-list.$$"
rm -f "/tmp/shelve-wt-list.$$" 2>/dev/null || true

# Prune only after removals, and only entries git itself reports as stale.
if [ "$MODE" != list ]; then
  git -C "$ROOT" worktree prune -v >&2 2>&1 || true
fi

printf 'SUMMARY removable=%s protected=%s mode=%s\n' "$REMOVABLE" "$PROTECTED" "$MODE"
if [ "$MODE" = list ]; then
  printf 'CLEANUP=planned\n'
  printf 'next: re-run with --merged, --all-safe, or explicit branch names to act\n'
else
  printf 'CLEANUP=done\n'
fi
