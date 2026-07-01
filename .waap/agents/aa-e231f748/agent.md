+++
creation_date = 2026-07-01T15:55:39Z
status = "completed"
session_id = "732afa44-29b7-4a48-b128-2eba68a68376"
system = "claude"
+++

# Purpose

You are a waap developer agent. Implement the functionality described in `.waap/tickets/tt-rename-repo-root-flag-to-waap-root/ticket.md`.

# Workflow

1. Keep a work log at `.waap/agents/${agent_id}/work_log.md`, appending as you work.
2. Read the ticket and the relevant source. The root-resolution work it depends on is already merged to main; rebase onto latest main before starting.
3. If the ticket is already `completed` or `abandoned`, stop without code changes.
4. Mark the ticket `in-progress` before editing code: `waap ticket update --ticket-id tt-rename-repo-root-flag-to-waap-root --set-status in-progress`.
5. Hard-rename the CLI flag `--repo-root` to `--waap-root` (no hidden alias). Grep the whole repo for `--repo-root` and `repo-root` to catch every reference (src, tests, doc comments, `.agents/skills/waap/SKILL.md`, role templates). Do not rename historical `.waap/` records.
6. Add/adjust tests so passing `--repo-root` is rejected and `--waap-root` works with the same default and resolution.
7. Run the required checks: `cargo clippy --all-targets -- -D warnings`, `cargo fmt --check`, `cargo test`, `cargo run -- check`, and `cargo run -- --help` (must show `--waap-root`, not `--repo-root`).
8. Rebase your branch onto the latest `main`, then merge with `git merge --ff-only`, resolving conflicts as needed.
9. Mark the ticket `completed` only after the code is merged and checks pass. `waap agent run` marks this agent `completed` automatically on successful exit, so do not set your own agent status.

# Parallel Work

Other agents or the user may be editing the repository concurrently. Do not revert or overwrite unrelated work. `waap agent run` prepares and removes your worktree; do not manage worktrees yourself. Commit on your branch and merge to main.

# Commit Guidance

Include both `${agent_id}` and the ticket id in commit messages.
