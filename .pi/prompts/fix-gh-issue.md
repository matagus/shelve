---
description: Fix a GitHub issue end-to-end — preflight, verdict, human gate, reproduce, smallest diff, CI-parity gate, release smoke, PR, checks. Each mechanical phase is one scripts/ call.
argument-hint: "<issue-number> [repo]"
model: litellm/qwen3.8-max
thinking: off
---

# Fix GitHub issue end-to-end

`$1` = issue number. `$2` non-empty → every `gh` call carries `-R $2`; empty → no `-R` flag at all. `$1` empty → `gh issue list --limit 20`, show it, stop.

## Budget: 4 script calls + ≤16 other tool calls

| Phase | Call | Verdict key |
| --- | --- | --- |
| 0·1·4 truth, sync main, worktree | `scripts/preflight.sh $1` | `PREFLIGHT=ok\|blocked` |
| 7 fmt→clippy→check→build→test→alloc, steps read from `ci.yaml` | `scripts/gate.sh "$WT"` | `GATE=PASS\|FAIL`, `failed_steps=` |
| 8 release binary, SIGPIPE, BASE↔NEW differential | `scripts/smoke.sh "$WT" "$ROOT"` | `SMOKE=PASS\|FAIL`, `DIFF=ok\|fail\|build-failed\|skipped` |
| 9·10 commit, push, PR, checks, issue comment | `scripts/ship.sh "$WT" $1 --title-file F --body-file B` | `SHIP=PASS\|INCOMPLETE\|FAIL`, `PR_URL=` |

Budget spent without the phase's exit condition → **STOP**: report the phase, what ran, what is missing. Never re-plan silently.

## Rules

- **Script output is ground truth**: the verdict key is the answer. Never re-run a green gate to see it render differently, never hand-roll a cargo/gh command a script covers, never write a JS or shell harness to drive a build tool.
- **Never claim a verification that did not run.** `DIFF=skipped` and `ISSUE_COMMENT=skipped` are skipped, not passed. Report failures you fixed.
- **Forbidden:** `worktree remove|prune`, `rm -rf`, `branch -D`, `checkout --`, `restore`, `reset --hard`, `clean`, `stash`, force-push, `--no-verify`, amending published commits, `add -A`, merging/closing a PR, rebasing `main`, `eval`, `cat -A` (BSD has none), `--manifest-path` before the subcommand, `gh pr create` without `--head <branch>`, `cd X && cmd; echo exit=$?`. Anything that could destroy uncommitted work → **STOP**.
- **Never `cd`.** Target paths explicitly: `git -C "$WT" …`, `cargo --manifest-path "$WT/Cargo.toml" test`. Use `NEW_BIN=` from smoke, never an assembled binary path. Infer dependency APIs by compiling, not by reading `~/.cargo/registry`.
- **Batch.** All read-only calls of a phase in ONE turn, tracker update included. No tracker-only turns.
- Scratch lives in `/tmp` and dies by `rm -f <explicit file>`. Nothing in the repo is ever deleted; worktree cleanup is the human's `scripts/wt-cleanup.sh`.

## Tracker

`task_list_set` once, in the preflight turn: one task per phase below, labels 3–15 words. Then one `task_list_update` per boundary, batched with a real call: entering `in_progress`, leaving `done` with `note` = verdict key and value (`GATE=PASS`). **STOP** → `blocked` + the gap; not applicable (docs-only diff → smoke) → `skipped` + reason. Never `task_list_set` again, never `task_list_clear`. Tools unavailable → a `- [ ]`/`- [x]` checklist at the same points.

## Phase 0·1·4 — Preflight (1 call)

`scripts/preflight.sh $1`, no slug: you have not read the title yet and a guessed name is worse than none. Keep `ROOT`, `BASE`, `WT`, `BRANCH`, `BIN` verbatim for the rest of the run. `PREFLIGHT=blocked` → **STOP** with the block; never work around it (no stash, no manual `worktree add`).

## Phase 2 — Verdict (1 turn, ≤4 read-only calls)

One turn: `gh issue view $1 --comments --json title,body,labels,state`; `gh pr list --search $1 --state open`; `rg -n` the cited symbols in `$WT` (line numbers must still match `BASE`); `git -C "$WT" log --oneline --grep $1`. **Quote `TITLE` verbatim from that output** — recalled from the issue number or an earlier session it is a hallucination, and everything downstream is named from it. Emit only this, ≤12 lines:

```
TITLE: …
STATE/LABELS: …
ASK: the acceptance criterion, quoted
EVIDENCE: path:line at BASE
ALREADY-FIXED: yes|no (commit | repro command)
DUPLICATE-PR: none | #n
VERDICT: fix | already-fixed | not-worth-it | needs-spec
PLAN: one implementation, ≤3 sentences
RISK: what this could break
BLOCKING: none | the decision needing the user
NEXT: phase
```

`VERDICT` other than `fix` → **STOP** with the reasoning; do not code. Alternatives belong in the PR body, not a private debate.

## Phase 3 — Human gate

Blocking = it would change the public API, the exit-code/behaviour contract, committed fixtures, or the diff size. Everything else is a default you state and take. Nothing blocking → print the defaults and continue **in the same turn**. Otherwise one `ask_user_question` call per decision (never a numbered prose list), 2–4 options, `(recommended)` first with its consequence in the description. Never ask what the repo, `ci.yaml` or the issue answers. Tool unavailable → one short prose block, then end the turn.

## Phase 5 — Reproduce

A failing test in `$WT`, or the captured artifact: exact command, actual vs expected stdout/stderr, exit code, kept as PR evidence. Cannot reproduce → **STOP** with what you tried.

## Phase 6 — Fix

Smallest diff that resolves the issue, its tests and any `tests/inputs/` + `tests/expected/` fixtures in the same change. No opportunistic refactors — park them as issue comments. Reuse `src/testing.rs`, `tests/golden.rs` and the `#[ignore]`d allocation reporter rather than re-deriving a harness.

## Phase 7 — Gate (1 call)

`scripts/gate.sh "$WT"`. Red → read `failed_steps=` and the `step=…:FAIL exit=N` lines, fix, re-run the **whole** script; its order is CI's, so there is nothing to restart from. `--only <step>` serves the editing loop — the last gate before smoke is always full. Never proceed red.

## Phase 8 — Smoke (1 call)

`scripts/smoke.sh "$WT" "$ROOT"`; the second argument is what makes the differential run, so omitting it downgrades the check. Docs-only diff → skip, mark the task `skipped`. `SMOKE=FAIL` or `DIFF=` not `ok` → fix as in Phase 6, then re-run gate **and** smoke.

## Phase 9·10 — Ship (1 call)

`/tmp/ship-title.txt`: one imperative line — it becomes both commit subject and PR title. `/tmp/ship-body.md`: H1 title, then problem → change → verification list → `Fixes #$1` → follow-ups; <50 words for a small diff, ≤100 for a large one. Then `scripts/ship.sh "$WT" $1 --title-file /tmp/ship-title.txt --body-file /tmp/ship-body.md`. Text travels in files, never argv. `PR_URL=` goes in the report verbatim, never reconstructed. `SHIP=INCOMPLETE|FAIL` → read `CHECKS=` or the failure, fix, re-run `gate.sh`, then re-run `ship.sh` (it reuses the PR). Never merge or close the issue. Leave the worktree, mark the last task `done`, confirm via `task_list_get` that nothing is `pending`, then report.

## Report — exactly 12 lines

```
PR: <PR_URL verbatim>
BRANCH: … | BASE: … | WT: …
SUMMARY: one line
FILES: …
VERDICT: <the Phase 2 VERDICT line>
GATE: GATE=… (fmt clippy check build test alloc)
SMOKE: SMOKE=… DIFF=…
SHIP: SHIP=… CHECKS=… ISSUE_COMMENT=…
DECISIONS: defaults taken | asked via ask_user: …
CALLS: n script + n other | STOPS: none | phase+reason
TRACKER: all done/skipped/blocked | FOLLOW-UPS: …
CLEANUP: worktree left in place — human runs `scripts/wt-cleanup.sh`, `--merged`, or `scripts/wt-cleanup.sh <branch>`
```
