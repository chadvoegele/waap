# Work log — aa-ffdee6e6

Ticket: tt-commitpaths-must-tolerate-no-op-commits-nothing-staged

## Goal
Make `commit_paths` in `src/git.rs` treat a no-op (nothing staged for the given
paths) as a successful no-op that returns the current HEAD, while still erroring
on genuine git failures.

## Changes
- Refactored `run_git` to share two helpers:
  - `git_command`: runs git and returns raw `Output` without treating a non-zero
    exit as an error, so callers can inspect the exit code.
  - `run_git_error`: builds the error for an unsuccessful invocation.
- In `commit_paths`, after `git add`, run `git diff --cached --quiet -- <paths>`
  via `git_command`. Exit 0 = nothing staged (skip commit), exit 1 = staged
  changes (commit), any other code = genuine failure (error). Always returns the
  current HEAD via `git rev-parse HEAD`.
- Kept the empty-`paths` guard.
- Added test `commit_paths_noop_returns_head_without_new_commit`.

## Validation
- cargo clippy --all-targets -- -D warnings: PASS
- cargo fmt --check: PASS
- cargo test: PASS (176 + 2 + 6 tests)
- cargo run -- check: PASS ("OK: .waap is valid")
