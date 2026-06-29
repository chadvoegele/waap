+++
title = "commit_paths must tolerate no-op commits (nothing staged)"
creation_date = 2026-06-29T20:07:40Z
status = "in-progress"
+++

# Problem

`commit_paths` in `src/git.rs` runs `git commit`, which exits non-zero ("nothing to commit") when the staged paths have no diff. `run_git` turns that non-zero status into a fatal error, so any waap state-commit that is a no-op aborts the whole command.

This surfaced live with `--system codex`: when the codex agent had already advanced the agent record to its target state (e.g., it ran `waap agent update` itself, or its merge already brought the record to `completed`), `finalize_codex_run` -> `mark_completed` writes the same `status = "completed"` and calls `commit_paths`, which finds nothing staged and fails with `git commit exited with exit status: 1` -> `failed to run agent`. The agent's actual work had already succeeded and merged; only the redundant terminal commit failed.

It is a latent robustness bug for all systems (claude/opencode included) — it simply hasn't surfaced there because those state writes always had a real diff. State commits should be idempotent.

# Desired Behavior

`commit_paths` treats "nothing to commit" as a successful no-op: when, after staging the given paths, there are no staged changes, it skips `git commit` and returns the current `HEAD` hash instead of erroring.

# Implementation Notes

- In `commit_paths` (`src/git.rs`), after the `git add -- <paths>`, check whether anything is staged for those paths (e.g. `git diff --cached --quiet -- <paths>`; exit 0 = no staged changes, exit 1 = changes). Only run `git commit` when there are staged changes. In both cases return the current `HEAD` from `git rev-parse HEAD`.
- Distinguish this benign case from genuine `git commit` failures (which must still error).
- `git diff --cached --quiet` uses exit code as signal, so it must be invoked without the generic `run_git` success-only wrapper (or `run_git` adjusted to allow callers to read a non-zero exit without treating it as an error).
- Keep the empty-`paths` guard.

# Acceptance Criteria

1. `commit_paths` with paths that have no staged changes returns `Ok(<current HEAD>)` and creates no new commit.
2. `commit_paths` with real changes still creates exactly one commit and returns its hash (unchanged behavior).
3. A genuine `git commit` failure (not "nothing to commit") still returns an error.
4. `mark_completed`/`mark_running`/`update_codex_session` no longer abort when the record is already in the target state.
5. Tests cover: no-op commit returns HEAD without a new commit; normal commit still works; empty paths still rejected.

# Validation

- `cargo clippy --all-targets -- -D warnings`
- `cargo fmt --check`
- `cargo test`
- `cargo run -- check`
