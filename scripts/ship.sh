#!/bin/sh
# shellcheck shell=sh
# ship.sh — Phases 9 and 10 of /fix-gh-issue in one call: commit, push, PR, checks.
#
# Usage:
#   scripts/ship.sh <worktree> <issue-number> --title-file <path> --body-file <path> [--dry-run]
#
# Both texts come from FILES, never from the command line. That is not style: a
# multi-line body passed as an argument has to survive quoting through the
# caller's shell wrapper, and every previous attempt at it produced either a
# mangled PR body or a turn spent re-quoting. Write the file, point at it.
#
# What this replaces: ~8 turns of staging, committing, pushing, `gh pr create`,
# `gh pr checks`, and re-polling. Two measured runs both got parts of it wrong —
# one opened the PR while the shell was still on main because --head was omitted,
# one polled CI once, saw nothing settled, and re-ran the whole check listing.
#
# Safety properties, in order of importance:
#   * refuses to run against the primary checkout (only .wt/<branch> worktrees)
#   * stages EXPLICIT paths; never `git add -A`, so scratch cannot ride along
#   * refuses an empty diff and refuses to push a branch whose HEAD != WT_HEAD
#   * never force-pushes, never amends, never merges, never closes
#   * leaves the worktree in place afterwards, and asserts that it did
#
# Exit status: 0 only when the PR exists and every required check passed.

set -eu

HERE=$(CDPATH='' cd -- "$(dirname -- "$0")" && pwd)
# shellcheck source=./lib.sh disable=SC1091
. "$HERE/lib.sh"

need_cmd git
need_cmd gh

WT=${1:-}
ISSUE=${2:-}
shift 2 2>/dev/null || die "usage: scripts/ship.sh <worktree> <issue-number> --title-file <path> --body-file <path>"

case "$ISSUE" in
  '' | *[!0-9]*) die "issue number must be digits: '$ISSUE'" ;;
esac

TITLE_FILE=''
BODY_FILE=''
DRY_RUN=0
while [ $# -gt 0 ]; do
  case $1 in
    --title-file) TITLE_FILE=${2:?--title-file needs a path}; shift 2 ;;
    --body-file) BODY_FILE=${2:?--body-file needs a path}; shift 2 ;;
    --dry-run) DRY_RUN=1; shift ;;
    *) die "unknown argument: $1" ;;
  esac
done

[ -n "$TITLE_FILE" ] || die "--title-file is required (see the header: text comes from files, not argv)"
[ -f "$TITLE_FILE" ] || die "title file not found: $TITLE_FILE"
[ -s "$TITLE_FILE" ] || die "title file is empty: $TITLE_FILE"
if [ -n "$BODY_FILE" ]; then
  [ -f "$BODY_FILE" ] || die "body file not found: $BODY_FILE"
fi

# --- Refuse anything that is not a dedicated issue worktree -------------------
#
# The prompt forbids running the shipping steps from the primary checkout, and
# the measured failure was real: opening a PR from `$ROOT` with no --head picked
# up whatever branch main's working copy happened to be on. Enforcing the path
# shape here means the guard does not depend on anyone remembering it.

COMMON_DIR=$(git -C "${WT:-.}" rev-parse --path-format=absolute --git-common-dir 2>/dev/null ||
  echo '')
GIT_DIR_ABS=$(git -C "${WT:-}" rev-parse --absolute-git-dir 2>/dev/null || echo '')
[ -n "$WT" ] && [ -d "$WT" ] || die "worktree directory not found: ${WT:-<none given>}"
WT=$(CDPATH='' cd -- "$WT" && pwd)

if [ "$GIT_DIR_ABS" = "$COMMON_DIR" ]; then
  # A plain clone: its .git dir IS the common dir. Linked worktrees have their
  # gitdir under <root>/.git/worktrees/<name>, so equality means we are standing
  # in the primary checkout and must not ship from here.
  die "refusing to ship from the primary checkout ($WT); pass the issue worktree path"
fi
case $WT in
  */.wt/*) : ;;
  *) die "refusing to ship from a path outside .wt/ (got $WT): use <repo>/.wt/issue-<n>-<slug>" ;;
esac

BRANCH=$(current_branch "$WT")
case $BRANCH in
  issue-$ISSUE*) : ;;
  *) die "branch '$BRANCH' does not match issue #$ISSUE; refusing to open a PR from it" ;;
esac

if git -C "$WT" remote get-url origin >/dev/null 2>&1; then
  REMOTE=origin
else
  die "no 'origin' remote configured in $WT"
fi
HEAD_SHA=$(head_sha "$WT")

# --- Stage explicit paths -----------------------------------------------------

# Every file is named explicitly; nothing is staged wholesale. `git add -A` is
# forbidden by the flow because it sweeps scratch files, other issues' output and
# stray binaries into the commit, so this enumerates the set and prints it before
# using it — a caller can see what is about to ship rather than trusting that it
# was filtered correctly.
#
# The tracked-file half is deliberately NOT `diff --name-only $BASE HEAD`: that
# range also lists whatever else sits between main and HEAD (an inherited branch
# commit, a rebase artifact), and staging those would credit unrelated changes to
# this issue. The worktree's OWN change is HEAD plus its working tree, so:
#   - dirty paths from `status`, which covers staged and unstaged edits alike
#   - untracked paths, minus anything ignored (--exclude-standard honours that)
# If the change is already committed, this set is empty and the script says so
# instead of inventing a commit.
BASE=$(git -C "$WT" merge-base HEAD "$REMOTE/main" 2>/dev/null || echo '')
[ -n "$BASE" ] || die "cannot find merge-base between $WT/HEAD and $REMOTE/main"

COMMITS_AHEAD=$(git -C "$WT" rev-list --count "$BASE"..HEAD)

CHANGED=$(
  {
    # Strip the two-character XY status prefix plus its space. `-b` (not `-z`) so
    # the value stays a plain path: with -c core.quotePath=false, non-ASCII names
    # are printed raw rather than as "\303\251" escapes that would not resolve.
    # `--untracked-files=no`: untracked files come from the next command, and
    # listing them twice would put one path on the staging list twice.
    git -C "$WT" -c core.quotePath=false status --porcelain --untracked-files=no |
      cut -c 3-
    git -C "$WT" ls-files --others --exclude-standard
  } | sed -e '/^$/d' | sort -u
)

if [ -z "$CHANGED" ] && [ "$COMMITS_AHEAD" -eq 0 ]; then
  printf 'SHIP=FAIL\n'
  log "nothing to ship: $WT has no uncommitted changes and no commits ahead of"
  log "$REMOTE/main ($BASE). Either the work was never done here, or the branch"
  log "was reset onto main. Refusing to open an empty PR."
  exit 1
fi

printf 'STAGED_FILES_BEGIN\n'
printf '%s\n' "$CHANGED"
printf 'STAGED_FILES_END\n'

if [ "$DRY_RUN" = 1 ]; then
  printf 'SHIP=dry-run\n'
  printf 'WT=%s\nBRANCH=%s\nBASE=%s\nHEAD=%s\n' "$WT" "$BRANCH" "$BASE" "$HEAD_SHA"
  exit 0
fi

if [ -n "$CHANGED" ]; then
  # pathspec-from-file instead of xargs: xargs splits on whitespace unless -0,
  # and building a NUL-delimited stream through a subshell is easy to get subtly
  # wrong for a filename containing a space. Git reads the list itself, verbatim.
  _paths=/tmp/shelve-ship-paths.$$
  printf '%s\n' "$CHANGED" >"$_paths"
  git -C "$WT" -c core.quotePath=false add --pathspec-from-file="$_paths" ||
    die "git add --pathspec-from-file failed"
  rm -f "$_paths" 2>/dev/null || true
else
  log "no uncommitted changes; shipping the $COMMITS_AHEAD existing commit(s)."
fi

# --- Commit -------------------------------------------------------------------

if ! git -C "$WT" diff --cached --quiet; then
  # Commit message from a file for the same reason the PR body is: the subject
  # line and paragraph breaks survive `-m` poorly once anyone quotes them.
  _msg=/tmp/shelve-ship-commit-msg.txt
  {
    printf '%s\n\n' "$(sed -n '1p' "$TITLE_FILE")"
    printf 'Refs #%s\n\n' "$ISSUE"
    if [ -n "$BODY_FILE" ]; then
      # Drop the PR body's own H1 (it duplicates the title) and keep the rest.
      tail -n +2 "$BODY_FILE"
    fi
  } >"$_msg"
  git -C "$WT" commit -F "$_msg" >&2 2>&1 || die "git commit failed"
  HEAD_SHA=$(head_sha "$WT")
  printf 'COMMIT=%s\n' "$HEAD_SHA"
else
  printf 'COMMIT=%s\n' "$HEAD_SHA"
  log "no staged changes to commit; shipping existing HEAD."
fi

# --- Push ---------------------------------------------------------------------

# No --force-with-lease, no -f: a rejected non-fast-forward push means someone
# else moved this branch, and that is a human decision, not something to paper
# over. Surface it and stop.
if ! git -C "$WT" push -u "$REMOTE" "$BRANCH" >&2 2>&1; then
  printf 'SHIP=FAIL\npush=rejected\n'
  log "push rejected. If the remote branch moved, reconcile manually — this script"
  log "will not force-push, because doing so discards someone else's commits."
  exit 1
fi
printf 'PUSH=ok branch=%s head=%s\n' "$BRANCH" "$(short_sha "$WT")"

# --- Pull request -------------------------------------------------------------
#
# `--head <branch>` is mandatory, not redundant. Without it gh infers the head
# from the branch of the directory it runs in, so creating the PR from anywhere
# other than $WT silently targets the wrong branch — which is how one measured
# run opened a PR for main while working on an issue branch.

REPO_SLUG=$(repo_slug "$WT") || die "cannot derive owner/repo from origin in $WT"
printf 'REPO=%s\n' "$REPO_SLUG"
PR_TITLE=$(sed -n '1p' "$TITLE_FILE")
[ -n "$PR_TITLE" ] || die "title file has no first line: $TITLE_FILE"

# Reuse an existing PR rather than opening a second one. `gh pr list` exits 0 with
# an empty array when nothing matches, so the test is on the payload, not the code.
EXISTING_JSON=$(gh pr list --repo "$REPO_SLUG" --head "$BRANCH" --state all \
  --json number,url,state 2>/dev/null || echo '')
PR_URL=''
PR_NUMBER=''
if printf '%s' "$EXISTING_JSON" | grep -q '"url"'; then
  # The URL is the only field worth parsing precisely, and it is also where the
  # number lives. Taking the number by scanning the JSON for "the first integer"
  # would return whichever field happened to come first in gh's output order —
  # silently wrong whenever that ordering changes.
  PR_URL=$(printf '%s' "$EXISTING_JSON" | grep -Eo 'https://[^"]*pull/[0-9]+' | head -n 1)
  PR_NUMBER=${PR_URL##*/}
  [ -n "$PR_URL" ] || die "gh listed a PR for $BRANCH but no URL could be extracted from its output"
  printf 'PR_EXISTS=yes\n'
  log "a PR already exists for $BRANCH; reusing it instead of opening a second one."
fi

if [ -z "$PR_URL" ]; then
  # Explicit argv, no shell string: the title and body are author-controlled text
  # and must reach gh as single arguments whatever they contain.
  # `if ! _out=$(cmd)` captures the command's status directly: assigning from a
  # command substitution and then reading `$?` reports the status of the
  # assignment, which is what turned an earlier false PASS into a real one.
  if [ -n "$BODY_FILE" ]; then
    if _pr_out=$(gh pr create --repo "$REPO_SLUG" --head "$BRANCH" --base main \
      --title "$PR_TITLE" --body-file "$BODY_FILE" 2>&1); then
      _pr_rc=0
    else
      _pr_rc=$?
    fi
  elif _pr_out=$(gh pr create --repo "$REPO_SLUG" --head "$BRANCH" --base main \
    --title "$PR_TITLE" --fill-first 2>&1); then
    _pr_rc=0
  else
    _pr_rc=$?
  fi
  printf '%s\n' "$_pr_out" >&2
  if [ "$_pr_rc" -ne 0 ]; then
    printf 'SHIP=FAIL\npr=create-failed exit=%s\n' "$_pr_rc"
    exit 1
  fi
  # gh prints the new URL on stdout; keep it verbatim rather than reconstructing
  # it, since a reconstructed URL is how a wrong repo or number gets reported.
  PR_URL=$(printf '%s' "$_pr_out" | grep -Eo 'https://[^[:space:]]*pull/[0-9]+' | head -n 1)
  [ -n "$PR_URL" ] || die "gh reported success but no PR URL was found in its output"
  PR_NUMBER=${PR_URL##*/}
  printf 'PR_CREATED=yes\n'
fi
printf 'PR_URL=%s\n' "$PR_URL"
printf 'PR_NUMBER=%s\n' "$PR_NUMBER"

# --- Checks -------------------------------------------------------------------
#
# `gh pr checks` exits non-zero while anything is still pending, so a single call
# cannot distinguish "green" from "not finished yet". One run measured that as a
# turn spent re-polling by hand. This loops until every check reports a terminal
# state, bounded so a hung required check cannot hang the pipeline forever.

# `_checks` is read after the loop, so it has to exist even when the very first
# iteration times out at zero seconds (a TIMEOUT of 0 is legal here): under
# `set -u` an uninitialised read would abort instead of reporting the verdict.
_checks=''
TIMEOUT=${SHIP_CHECKS_TIMEOUT:-600}
INTERVAL=${SHIP_CHECKS_INTERVAL:-20}
WAITED=0
CHECKS_STATE=unknown
while [ "$WAITED" -lt "$TIMEOUT" ]; do
  _checks=$(gh pr checks "$PR_NUMBER" --repo "$REPO_SLUG" 2>/dev/null || true)
  if [ -z "$_checks" ]; then
    # No rows at all means the checks have not been registered yet, which is
    # normal in the seconds after a push. Keep waiting; do not report PASS.
    CHECKS_STATE=pending
  elif printf '%s' "$_checks" | grep -qiE '(fail|error|✗|x )'; then
    CHECKS_STATE=failing
    break
  elif printf '%s' "$_checks" | grep -qiE '(pending|in_progress|queued|running|⏳|○)'; then
    CHECKS_STATE=pending
  elif printf '%s' "$_checks" | grep -qiE '(pass|✓|success)'; then
    CHECKS_STATE=passing
    break
  else
    # Rows exist but match none of the known states. Treat that as pending rather
    # than breaking out: gh has changed its status vocabulary before, and a parse
    # miss must not be reported as either PASS or FAIL. The timeout bounds it.
    CHECKS_STATE=unknown
  fi
  printf 'checks_waited=%s state=%s\n' "$WAITED" "$CHECKS_STATE" >&2
  sleep "$INTERVAL"
  WAITED=$((WAITED + INTERVAL))
done

printf '%s\n' "$_checks" >&2
printf 'CHECKS=%s waited=%ss\n' "$CHECKS_STATE" "$WAITED"

# --- Issue comment ------------------------------------------------------------

if [ -n "$BODY_FILE" ] && [ "$CHECKS_STATE" = passing ]; then
  # Comment on the ISSUE, not the PR: the reporter follows the issue. Skipped when
  # checks are red, because "fixed, PR here" on a failing build is a claim that
  # will be read as false within the hour.
  _cmsg=/tmp/shelve-ship-issue-comment.md
  {
    printf 'Fixed in %s.\n\n' "$PR_URL"
    # Fenced code block around the title so backticks or dollar signs in a PR
    # subject cannot turn into live template syntax in the issue body.
    # shellcheck disable=SC2016  # the fence is literal markdown, not a format string
    printf '```\n%s\n```\n' "$PR_TITLE"
  } >"$_cmsg"
  if gh issue comment "$ISSUE" --repo "$REPO_SLUG" --body-file "$_cmsg" >&2 2>&1; then
    printf 'ISSUE_COMMENT=post\n'
  else
    printf 'ISSUE_COMMENT=failed exit=%s\n' $?
  fi
else
  if [ "$CHECKS_STATE" != passing ]; then
    # Deliberate: commenting "fixed, PR here" while CI is red is a claim the
    # reporter will read as false within the hour.
    printf 'ISSUE_COMMENT=skipped reason=checks-%s\n' "$CHECKS_STATE"
  else
    printf 'ISSUE_COMMENT=skipped reason=no-body-file\n'
  fi
fi

# --- Final assertion ----------------------------------------------------------
#
# The flow's last rule is that the worktree survives for review. Asserting it here
# turns that rule into an observable fact instead of an assumption: if anything in
# this script (or a hook it triggered) removed $WT, the report says so.

if [ -d "$WT" ] && [ "$(head_sha "$WT")" = "$HEAD_SHA" ]; then
  printf 'WT_STILL_PRESENT=yes\n'
else
  printf 'WT_STILL_PRESENT=no\n'
  log "WARNING: $WT is missing or moved off $HEAD_SHA despite never being touched here."
fi
printf 'WT=%s\nBRANCH=%s\nBASE=%s\nHEAD=%s\n' "$WT" "$BRANCH" "$BASE" "$HEAD_SHA"
# The removal command is printed, never run: deleting the workspace is the
# reviewer's call, and the flow forbids the agent doing it at any point, including
# after CI goes green. MAIN_ROOT is derived from the common git dir (<root>/.git),
# which is the one path that identifies the primary checkout from inside a linked
# worktree.
MAIN_ROOT=${COMMON_DIR%/.git}
printf 'MAIN_ROOT=%s\n' "$MAIN_ROOT"
printf 'REMOVE_WITH=scripts/wt-cleanup.sh %s\n' "$BRANCH"

case $CHECKS_STATE in
  passing) printf 'SHIP=PASS\n'; exit 0 ;;
  *) printf 'SHIP=INCOMPLETE checks=%s\n' "$CHECKS_STATE"; exit 2 ;;
esac
