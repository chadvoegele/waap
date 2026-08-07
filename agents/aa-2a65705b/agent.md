+++
creation_date = 2026-06-29T15:06:23Z
status = "completed"
session_id = "4ebb585e-aeaf-4d68-b147-89e41ff23889"
system = "claude"
+++

# Purpose
Implement code for `tt-linearize-agent-run-git-history-with-rebase-and-fast-forwa-d778`.

# Instructions
Implement the functionality described in `/.waap/tickets/tt-linearize-agent-run-git-history-with-rebase-and-fast-forwa-d778/ticket.md`. Inspect `src/agent/run.rs` and `src/git.rs` before choosing an implementation; make the smallest correct change.

`waap agent run` already prepares a git worktree for you and runs you inside it, so do NOT create or remove a worktree yourself. Make your changes in the current working directory and commit them on your branch.

Follow the Developer Validations in `AGENTS.md`: run `cargo clippy --all-targets -- -D warnings`, `cargo fmt --check`, and `cargo test` and ensure all pass before committing. Also run `cargo run -- check`.

Before you start: `waap ticket update --ticket-id tt-linearize-agent-run-git-history-with-rebase-and-fast-forwa-d778 --set-status in-progress`.

When your code is validated, merge it to the `main` branch: rebase your branch onto the latest `main` and use a fast-forward (`--ff-only`) merge to keep history linear, resolving conflicts as needed. Your changes are lost if you do not merge to main. Include your agent id and the ticket id in your commit message.

If the ticket is already completed or abandoned, complete your `/goal` without code changes.

After your code is merged and validated:
1. mark the ticket completed: `waap ticket update --ticket-id tt-linearize-agent-run-git-history-with-rebase-and-fast-forwa-d778 --set-status completed`
2. mark this agent completed: `waap agent update --agent-id ${agent_id} --set-status completed`
