+++
creation_date = 2026-06-29T15:15:42Z
status = "ready"
+++

# Purpose
Implement code for `tt-mark-agent-completed-on-successful-agent-run-exit`.

# Instructions
Implement the functionality described in `/.waap/tickets/tt-mark-agent-completed-on-successful-agent-run-exit/ticket.md`. Inspect `src/agent/run.rs` (especially `run_in_agent_worktree`, `run_agent_claude`, `run_agent_opencode`, `mark_running`, `exit_code_from_status`) before choosing an implementation; make the smallest correct change.

`waap agent run` already prepares a git worktree for you and runs you inside it, so do NOT create or remove a worktree yourself. Make your changes in the current working directory and commit them on your branch.

Follow the Developer Validations in `AGENTS.md`: run `cargo clippy --all-targets -- -D warnings`, `cargo fmt --check`, and `cargo test` and ensure all pass before committing. Also run `cargo run -- check`.

Before you start: `waap ticket update --ticket-id tt-mark-agent-completed-on-successful-agent-run-exit --set-status in-progress`.

When your code is validated, merge it to the `main` branch: rebase your branch onto the latest `main` and use a fast-forward (`--ff-only`) merge to keep history linear, resolving conflicts as needed. Your changes are lost if you do not merge to main. Include your agent id and the ticket id in your commit message.

If the ticket is already completed or abandoned, complete your `/goal` without code changes.

After your code is merged and validated, mark the ticket completed: `waap ticket update --ticket-id tt-mark-agent-completed-on-successful-agent-run-exit --set-status completed`. (Per this ticket, waap itself now marks the agent status completed on a successful exit, so you do not need to mark your own agent status.)
