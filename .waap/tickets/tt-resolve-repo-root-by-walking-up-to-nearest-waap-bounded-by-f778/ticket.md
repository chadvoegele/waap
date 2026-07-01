+++
title = "Resolve repo root by walking up to nearest .waap bounded by git root; validate --repo-root"
creation_date = 2026-07-01T15:26:17Z
status = "in-progress"
depends_on = ["tt-add-waap-init-command-require-waap-for-mutating-commands"]
+++

# Problem

waap takes `--repo-root` (default `.`, see `src/cli.rs`) and never validates it or walks to find the real project root. Running from a subdirectory silently operates on `./.waap` (usually nonexistent), and pointing `--repo-root` at a non-project directory fails confusingly deep inside a command.

This ticket depends on `tt-add-waap-init-command-require-waap-for-mutating-commands` (the `waap init` command and the "must be initialized" errors it introduces). Keep error messages consistent with that ticket.

# Desired Behavior

## Git-root resolution is a precondition (its error takes precedence)

Determine the **git root** first: from the starting directory, walk up to the nearest ancestor containing a `.git` entry, accepting `.git` as either a **file** (linked worktrees, submodules) or a **directory**. If none is found, error `not inside a git repository` and stop. This check happens before any `.waap` lookup, so its error takes precedence over "no waap project".

Detect the git root by finding the nearest `.git` entry directly. Do **not** shell out to `git rev-parse --show-toplevel`: from a linked worktree that returns the main repo's toplevel, which is the wrong boundary — an agent running in `worktrees/<id>` must resolve to its own worktree checkout, not the main repo.

## Resolution when `--repo-root` is NOT passed

Starting at the current directory, walk up looking for a directory containing `.waap/`, **bounded by and inclusive of the git root** — never search above the git root.

- First `.waap/` found -> that directory is the repo root.
- If none is found down to and including the git root -> error `no waap project found; run 'waap init'`.

Because the git root is the ceiling of the walk, any resolved `.waap/` is guaranteed to live inside a git repository, so later state commits always have a git repo to operate in.

## Resolution when `--repo-root X` IS passed (explicit, no walking)

`--repo-root` names the root exactly; do not walk up from it.

- `X` does not exist -> error.
- `X` is not inside a git repository -> error `not inside a git repository`.
- `X` does not directly contain `.waap/` -> error `no waap project at <X>; run 'waap init' or omit --repo-root`.
- `X` contains `.waap/` and is inside a git repo -> use `X`.

# Cases (must hold)

- cwd is the project root with `.waap` -> that dir.
- cwd is a subdir below a `.waap` in the same repo -> the nearest ancestor `.waap`.
- Two projects in one repo (`lab/.waap` and `lab/wiki/.waap`): from `lab` -> `lab`; from `lab/wiki` -> `lab/wiki` (nearest wins).
- New repo with `.git` but no `.waap`, and a stray `.waap` in an ancestor ABOVE the git root -> error `run 'waap init'`; the ancestor `.waap` is never used (bounded by git root).
- Not in a git repo anywhere up the tree -> `not inside a git repository`.
- Linked worktree (`.git` is a file) whose checkout has its own `.waap` -> resolves to the worktree root.
- Only `.waap` at the git root with intervening subdirs -> resolves to the git root.

# Suggested Implementation

- Add a resolver (e.g. in a small module or `src/record.rs`) returning the resolved root or a descriptive error, taking the starting dir and the optional explicit `--repo-root`.
- Wire it into `app::run` so every command uses the resolved root; keep `--output-format json` stdout clean (errors to stderr, non-zero exit).
- Canonicalize paths before comparing against the git-root boundary to avoid symlink/relative-path edge cases.
- The walk is O(depth) stat calls — do not scan subtrees.

# Acceptance Criteria

1. With no `--repo-root`, commands resolve to the nearest ancestor `.waap` bounded by the git root; all "Cases" above hold.
2. Not being inside a git repo yields `not inside a git repository`, and this check precedes the "no waap project" error.
3. A `.waap` above the git root is never selected.
4. `--repo-root X` is treated as exact and is validated for existence, git membership, and a direct `.waap/`; each failure has a distinct, actionable error.
5. `.git` is accepted as both a file and a directory (worktrees resolve to their own checkout, not the main repo).
6. `--output-format json` stdout stays clean; resolution errors go to stderr with a non-zero exit.
7. Tests cover each bullet in "Cases", plus the `--repo-root` validation failures.

# Validation

- `cargo clippy --all-targets -- -D warnings`
- `cargo fmt --check`
- `cargo test`
- `cargo run -- check`
