+++
creation_date = 2026-06-29T14:24:57Z
status = "completed"
session_id = "9c842b84-2c23-428c-aafa-414654d3e898"
system = "claude"
+++

# Purpose
Implement code for `tt-move-worktree-management-to-agent-run`.

# Instructions
Your role is to implement the functionality described in `/.waap/tickets/tt-move-worktree-management-to-agent-run/ticket.md`. After implementing the code, write adequate unit tests, and end-to-end tests if appropriate. Once the code is tested, merge it to the main branch, resolving conflicts as necessary.

`waap agent run` already prepares a git worktree for you and runs you inside it, so you do NOT need to create or remove a worktree yourself. Simply make your changes in the current working directory, commit them on your branch, and merge to main.

Before you start, mark your ticket as in-progress with `waap ticket update --ticket-id tt-move-worktree-management-to-agent-run --set-status in-progress`.

Run the validation checks in the ticket (e.g. `cargo fmt`, `cargo test`, `cargo run -- check`). Do not merge unless they pass.

Include your agent id and the ticket id you worked on in your commit message. Use a fast-forward merge when possible to keep a linear history.

If the ticket is already completed or abandoned, complete your `/goal` without code changes.

After your code is merged and tested,
1. mark your ticket as 'completed' with `waap ticket update --ticket-id tt-move-worktree-management-to-agent-run --set-status completed`.
1. mark your status as 'completed' with `waap agent update --agent-id ${agent_id} --set-status completed`.
