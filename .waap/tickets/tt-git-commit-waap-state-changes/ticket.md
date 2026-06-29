+++
title = "Git commit waap state changes"
creation_date = 2026-06-27T16:46:24Z
status = "completed"
+++

# Spec Reference

Lines 148-221 of /specs/spec.md

# Description

Make waap commit its own state file changes after successful mutating commands. This keeps ticket and agent state durable in git and avoids a manual commit step.

# Requirements

- Commit ticket files after
    - `waap ticket new`
    - `waap ticket update`
- Commit agent files after
    - `waap agent new`
    - `waap agent update`
    - `waap agent run`
    - `waap agent stop`
- Stage and commit only the files modified by the waap command; do not include unrelated user or agent worktree changes.
- Use concise, deterministic commit messages that identify the command and record id, such as ticket id or agent id.
- Respect `--repo-root` when running git commands and resolving changed file paths.

# Acceptance Criteria

1. Each successful `ticket new`, `ticket update`, `agent new`, `agent update`, `agent run`, and `agent stop` command creates exactly one git commit for the waap state files changed by that command.
2. Commits do not stage unrelated changes already present in the repository.
3. Failed git commits return a non-zero command result with a useful diagnostic while leaving the waap state update intact.
4. JSON and human-readable command output indicate the commit hash or that the commit failed.
5. Tests cover commit creation, selective staging, commit failure handling, and `--repo-root` behavior for the affected commands.

# Validation

- `cargo fmt`
- `cargo test`
- `cargo run -- check`
