+++
creation_date = 2026-07-01T15:37:21Z
status = "ready"
+++

# Purpose

You are a waap developer agent. Implement the functionality described in `.waap/tickets/tt-resolve-repo-root-by-walking-up-to-nearest-waap-bounded-by-f778/ticket.md`.

# Workflow

1. Keep a work log at `.waap/agents/${agent_id}/work_log.md`, appending as you work.
2. Read the ticket and the relevant source (`src/cli.rs`, `src/app.rs`, `src/record.rs`, `src/git.rs`). Note that `waap init` and the "require initialized project" behavior already landed on main (ticket tt-add-waap-init-command-require-waap-for-mutating-commands); build on it and keep error messages consistent.
3. If the ticket is already `completed` or `abandoned`, stop without code changes.
4. Mark the ticket `in-progress` before editing code: `waap ticket update --ticket-id tt-resolve-repo-root-by-walking-up-to-nearest-waap-bounded-by-f778 --set-status in-progress`.
5. Use the smallest correct change that satisfies the ticket's acceptance criteria. Detect the git root by finding the nearest `.git` entry yourself (file or dir); do NOT use `git rev-parse --show-toplevel`.
6. Add unit tests covering every bullet in the ticket's "Cases" section plus the `--repo-root` validation failures.
7. Run the required checks: `cargo clippy --all-targets -- -D warnings`, `cargo fmt --check`, `cargo test`, `cargo run -- check`.
8. Rebase your branch onto the latest `main`, then merge with `git merge --ff-only`, resolving conflicts as needed.
9. Mark the ticket `completed` only after the code is merged and checks pass. `waap agent run` marks this agent `completed` automatically on successful exit, so do not set your own agent status.

# Parallel Work

Other agents or the user may be editing the repository concurrently. Do not revert or overwrite unrelated work. `waap agent run` prepares and removes your worktree; do not manage worktrees yourself. Commit on your branch and merge to main.

# Commit Guidance

Include both `${agent_id}` and the ticket id in commit messages.
