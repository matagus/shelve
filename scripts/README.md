# Pipeline scripts

Four checked-in shell scripts carry the mechanical part of `/fix-gh-issue`, plus
one for human-side cleanup. They exist because in two measured runs, **3–6% of
wall time was actual command execution** and the rest was the model generating
tokens to decide what to run and then disbelieving what came back. Roughly 30–40
turns per issue went into re-deriving things that do not change between issues.

| Script | Replaces | One call does |
| --- | --- | --- |
| `preflight.sh` | Phases 0 + 1 + 4 | clean-tree assert, `fetch --prune`, ff-only main sync, worktree create, `WT_HEAD == BASE` proof |
| `gate.sh` | Phase 7 | fmt → clippy (pedantic, JSON) → check → build → test → allocation budgets, step list read from `ci.yaml` |
| `smoke.sh` | Phase 8 | release build, process-level checks, SIGPIPE/closed stdout, BASE↔NEW byte differential |
| `ship.sh` | Phases 9 + 10 | explicit-path staging, commit, push, PR with `--head`, bounded check-settle loop, issue comment |
| `wt-cleanup.sh` | — | human-invoked removal of accumulated `.wt/` worktrees |

## Conventions every script obeys

**POSIX sh only.** No bashisms (`[[`, `${var:0:7}`, `$(( ))` arrays), no GNU-only
flags (`cat -A`, `readlink -f`, `date -d`), nothing that breaks under `set -eu`.
They run identically under `/bin/sh` (bash 3.2 on macOS), dash and busybox ash.
Verified with `shellcheck -s sh` and `dash -n`.

**No `eval` of agent-authored text.** The one `eval` in the tree (`gate.sh`) turns
a `run:` string *read out of ci.yaml* into an argument vector, with no expansion —
so a literal `$HOME` in a CI command stays literal. Everything else is argv. This
matters operationally as well as stylistically: `cc-safety-net` blocks commands it
cannot verify, and a script whose meaning depends on a second parsing pass gets
blocked or silently mis-executed.

**Machine-readable stdout, prose on stderr.** Every script emits `KEY=value` lines
and `<step>:ok` / `<step>:FAIL exit=N` markers. Verdicts are terminal lines
(`PREFLIGHT=`, `GATE=`, `SMOKE=`, `DIFF=`, `SHIP=`). A caller can parse the result
without reading a paragraph, and cannot mistake prose for a verdict.

**Exit status matches the verdict.** Non-zero whenever anything failed, including
when the failure is "this check could not be performed".

## Shared build cache

`.cargo/config.toml` sets `target-dir = "../.shared-target"`, resolved relative to
the config file, so every worktree of a checkout builds into one directory beside
it. Cargo's default is a per-checkout `target/`, which meant phase 8 paid a cold
release build in every fresh worktree and threw the artifacts away afterwards.
`.wt/` and `.shared-target/` are both gitignored.

`gate.sh` and `smoke.sh` print the absolute path they resolved, rather than
assuming the config applied.

## Failure modes these scripts close

Each of these was reproduced while writing them, not inferred from the analysis —
several were bugs in earlier versions of these same scripts.

- **A gate that reports success on a failing check.** `if cmd; then return 0; fi;
  return $?` returns **0**, because `$?` is reset by the failing test itself. That
  shape turned a red clippy run into `GATE=PASS` here until it was replaced with
  `_st=0; cmd || _st=$?`. Verified against dash and bash 3.2.
- **A differential comparing a binary with itself.** Two hazards, both live: the
  shared target dir gives both builds the same artifact path, and cargo decides
  what to rebuild from **mtimes**, which `tar`/`cp` preserve — so a copied base
  checkout looked older than another checkout's cached artifact and cargo reused
  it. A probe that changed every group header still reported `DIFF=ok`. Fixed by
  building each side into its own `CARGO_TARGET_DIR` named after the revision it
  holds, and copying the binaries out immediately.
- **A vacuous pass on absent input.** With a missing fixture, `got=[]` equals
  `expected=[]` and the ordering check passes. Now reported as `SKIP` with a
  reason, never as `PASS`.
- **Substring collisions in config parsing.** `Check` is a substring of `Check
  formatting`, and a YAML comment above a step mentions both. Matching on
  substring made `gate.sh` run `cargo fmt --all --check` and report it as the
  *Check* step passing. Step lookup is now anchored on `- name: <exact>` with
  comments stripped.
- **Nested JSON that grep cannot read.** With `--message-format=json`, a
  diagnostic's `level` is not where you expect it: the first `"level":"` on the
  line belongs to a child note (`help`), and the real top-level `error` appears
  later. Positional extraction reported `diagnostics=0` for a red gate. The level
  now comes from the rendered text, which is unambiguous.
- **`$@` does not escape a function.** POSIX shells report the *parent's*
  positional params after a function returns, so a helper that "returns an argv"
  silently re-ran the previous step. Splitting is done in place, immediately
  before exec.

## Usage

```sh
# 1. sync main, create the worktree, prove HEAD == BASE
scripts/preflight.sh 49 avoid-allocating-group-key
#    -> ROOT= HEAD0= BRANCH0= BASE= WT= BRANCH= CARGO_TARGET_DIR= BIN=

# 2. every CI gate, in CI's order, with CI's flags
scripts/gate.sh "$WT"                      # full pipeline
scripts/gate.sh "$WT" --only clippy        # one step
scripts/gate.sh "$WT" --profile release

# 3. the real binary, plus the byte differential against the merge base
scripts/smoke.sh "$WT" "$ROOT"

# 4. commit, push, PR, watch checks
printf 'fix: stop allocating the group key\n' > /tmp/title.txt
printf '# PR body\n\n...' > /tmp/body.txt
scripts/ship.sh "$WT" 49 --title-file /tmp/title.txt --body-file /tmp/body.txt
scripts/ship.sh "$WT" 49 --title-file /tmp/title.txt --dry-run   # inspect first

# 5. when the accumulated worktrees are yours to clear
scripts/wt-cleanup.sh            # list only
scripts/wt-cleanup.sh --merged   # remove what is already in main
```

Text always goes through **files**, never argv: a multi-line PR body passed as an
argument has to survive quoting through whatever wrapper the caller runs under,
and every attempt at that produced either a mangled body or a turn spent
re-quoting.

## Deliberate limits

- `gate.sh` reads `ci.yaml` with awk, not yq — yq is not a base install on macOS
  or most CI images, and requiring it would make the parity check depend on a tool
  missing exactly when the agent needs it. It handles single-line `run:` bodies and
  `run: |` blocks (written out and executed with `sh -e`).
- `smoke.sh` does **not** re-implement the `tests/cli.rs` matrix. That suite covers
  help, version, column bounds, multi-file, stdin, empty input, malformed input,
  CRLF, BOM, unicode and wide rows. Only the three things a unit test cannot reach
  are checked here: the release artifact, SIGPIPE at the process boundary, and the
  BASE↔NEW differential.
- The differential sweeps columns `1..5`, matching `MAX_SWEPT_COLUMNS` in
  `tests/golden.rs`, so the two agree by construction rather than by copy-paste.
- Nothing here removes a worktree except `wt-cleanup.sh`, which is documented as
  human-invoked. The flow's rule that the agent leaves the workspace intact is
  correct and these scripts do not weaken it; `ship.sh` even re-asserts at the end
  that `$WT` still exists at the expected SHA.
