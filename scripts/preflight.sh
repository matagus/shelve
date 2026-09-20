#!/bin/sh
# shellcheck shell=sh
# preflight.sh — Phases 0, 1 and 4 of /fix-gh-issue in one call.
#
#   Phase 0: establish ground truth (ROOT, HEAD0, BRANCH0) and prove the tree clean
#   Phase 1: sync main with origin fast-forward-only, record BASE
#   Phase 4: create the issue worktree at BASE and prove WT_HEAD == BASE
#
# Everything those phases exist to guarantee arrives as one machine-readable
# block, so no caller has to interpret `git status` prose or re-run rev-parse
# three ways to believe a SHA. Before this script existed, the same three facts
# cost 6–13 turns per run, because each individual command's output looked
# plausible and none of them were checkable together.
#
# Usage:
#   scripts/preflight.sh <issue-number> [slug]
#
#   <issue-number>  required; names the branch and the worktree path
#   [slug]          optional kebab-case suffix, <= 40 chars, [a-z0-9-]. Omit it
#                   if you have not read the issue title yet: the branch is
#                   renameable, and a guessed slug is worse than no slug.
#
# Refuses to touch a dirty tree, refuses a non-fast-forward main, refuses to
# reuse an existing branch/worktree path. It never deletes anything: a stale
# worktree from a previous session is reported, and cleaning it up is a human
# decision (see scripts/wt-cleanup.sh).

set -eu

HERE=$(CDPATH='' cd -- "$(dirname -- "$0")" && pwd)
# shellcheck source=./lib.sh disable=SC1091
. "$HERE/lib.sh"

# Resolve the checkout from this script's own location, not $PWD: the caller may
# invoke it by absolute path from anywhere, and every fact below (ROOT, BASE, WT)
# has to refer to the same repository the scripts are checked into.
REPO_ROOT=$(CDPATH='' cd -- "$HERE/.." && pwd)

need_cmd git
need_cmd cargo

ISSUE=${1:-}
SLUG=${2:-}

case "$ISSUE" in
  '' | *[!0-9]*) die "usage: scripts/preflight.sh <issue-number> [slug] — issue number must be digits" ;;
esac

if [ -n "$SLUG" ]; then
  # Kebab-case only: the slug ends up in a branch name, a path, and a PR title.
  case "$SLUG" in
    *[!a-z0-9-]*) die "slug must be kebab-case ([a-z0-9-], no leading/trailing dash): $SLUG" ;;
  esac
  case "$SLUG" in
    -* | *- | '') die "slug must be kebab-case with no leading or trailing '-': $SLUG" ;;
  esac
  [ "${#SLUG}" -le 40 ] || die "slug too long (max 40 chars): $SLUG"
fi

BRANCH="issue-$ISSUE"
[ -n "$SLUG" ] && BRANCH="$BRANCH-$SLUG"

# Worktrees live under .wt/ inside the checkout, never in a tool-managed
# namespace outside it — the reviewer needs to `cd` there after the run.
WT="$REPO_ROOT/.wt/$BRANCH"

# --- Phase 0 -----------------------------------------------------------------

require_clean_tree "$REPO_ROOT"

ROOT=$REPO_ROOT
HEAD0=$(head_sha "$ROOT")
BRANCH0=$(current_branch "$ROOT")

# The flow branches off main. If the checkout is on a feature branch, `main` may
# be far behind HEAD0 and BASE would silently mean something different — say so
# rather than letting the caller assume.
MAIN_IS_HEAD0=no
[ "$BRANCH0" = main ] && MAIN_IS_HEAD0=yes

# Report a leftover worktree for this issue instead of failing on it: the caller
# may legitimately be resuming, and the two cases need different answers.
WT_PREEXISTING=no
BRANCH_PREEXISTING=no
git -C "$ROOT" show-ref --verify --quiet "refs/heads/$BRANCH" && BRANCH_PREEXISTING=yes
[ -e "$WT" ] && WT_PREEXISTING=yes

# --- Phase 1 -----------------------------------------------------------------

git -C "$ROOT" fetch --prune origin >&2 2>&1 || die "git fetch --prune origin failed"

# Fast-forward only. A merge or rebase here would rewrite history nobody asked
# for; if main cannot ff, that is a human reconciliation problem, so stop with
# the reason on stderr.
#
# `main` is updated with update-ref rather than `git pull --ff-only`: pull
# requires checking out main, and the caller may legitimately be standing on a
# feature branch (reported as MAIN_IS_HEAD0=no above). The two refs are equal
# after this line whenever the ancestry check passed.
LOCAL_MAIN=$(git -C "$ROOT" rev-parse refs/heads/main 2>/dev/null || echo missing)
REMOTE_MAIN=$(git -C "$ROOT" rev-parse refs/remotes/origin/main 2>/dev/null || echo missing)

if [ "$LOCAL_MAIN" = missing ] || [ "$REMOTE_MAIN" = missing ]; then
  die "cannot resolve local main ($LOCAL_MAIN) or origin/main ($REMOTE_MAIN)"
fi

if git -C "$ROOT" merge-base --is-ancestor "$LOCAL_MAIN" "$REMOTE_MAIN"; then
  # `--ff-only` on a branch we are not standing on still updates the ref.
  git -C "$ROOT" update-ref refs/heads/main "$REMOTE_MAIN"
  MAIN_SYNCED=ff
else
  log "local main ($LOCAL_MAIN) is not an ancestor of origin/main ($REMOTE_MAIN)."
  log "main has diverged or carries local commits; refusing to reconcile it automatically."
  exit 2
fi

BASE=$REMOTE_MAIN

# --- Phase 4 -----------------------------------------------------------------

if [ "$WT_PREEXISTING" = yes ] || [ "$BRANCH_PREEXISTING" = yes ]; then
  log "branch '$BRANCH' and/or worktree '$WT' already exists."
  log "If this is a resumed session, run gate/smoke against the existing WT."
  log "If it is stale, remove it deliberately: scripts/wt-cleanup.sh"
  printf 'PREEXISTING=branch=%s worktree=%s\n' "$BRANCH_PREEXISTING" "$WT_PREEXISTING"
  printf 'WT=%s\n' "$WT"
  printf 'BASE=%s\n' "$BASE"
  printf 'PREFLIGHT=blocked\n'
  exit 3
fi

git -C "$ROOT" worktree add "$WT" -b "$BRANCH" main >&2 2>&1 ||
  die "git worktree add failed for $WT"

WT_HEAD=$(head_sha "$WT")
if [ "$WT_HEAD" != "$BASE" ]; then
  log "worktree HEAD ($WT_HEAD) does not match BASE ($BASE) — refusing to continue."
  exit 4
fi

# Prove the shared target dir resolved from inside the worktree, not just from
# main: .cargo/config.toml is committed, so it should, but "should" is exactly
# the kind of claim this script exists to replace with an observation.
SHARED_TARGET=$(target_dir "$WT")

# --- Output ------------------------------------------------------------------

printf 'PREFLIGHT=ok\n'
printf 'ROOT=%s\n' "$ROOT"
printf 'HEAD0=%s\n' "$HEAD0"
printf 'HEAD0_SHORT=%s\n' "$(short_sha "$ROOT")"
printf 'BRANCH0=%s\n' "$BRANCH0"
printf 'MAIN_IS_HEAD0=%s\n' "$MAIN_IS_HEAD0"
printf 'MAIN_SYNCED=%s\n' "$MAIN_SYNCED"
printf 'BASE=%s\n' "$BASE"
printf 'BASE_SHORT=%s\n' "$(short_sha "$WT")"
printf 'WT=%s\n' "$WT"
printf 'BRANCH=%s\n' "$BRANCH"
printf 'WT_HEAD=%s\n' "$WT_HEAD"
printf 'WT_HEAD_EQ_BASE=yes\n'
printf 'CARGO_TARGET_DIR=%s\n' "$SHARED_TARGET"
printf 'BIN=%s\n' "$(pkg_name "$WT")"
