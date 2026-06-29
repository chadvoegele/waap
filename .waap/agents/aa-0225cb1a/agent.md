+++
creation_date = 2026-06-29T19:09:20Z
status = "ready"
+++

# Purpose
Implement code for `tt-add-agentsystemcodex-variant-and-cli-wiring`.

# Instructions
Implement the functionality described in `/.waap/tickets/tt-add-agentsystemcodex-variant-and-cli-wiring/ticket.md`, following the design in `/specs/codex-agent-system.md`. Inspect the referenced waap source and (where the ticket needs exact codex JSON-RPC names) `/home/cvoegele/code/github.com/openai/codex` before implementing. Make the smallest correct change.

`waap agent run` already prepares a git worktree for you and runs you inside it; do NOT create or remove a worktree yourself. Work in the current directory and commit on your branch.

Follow the AGENTS.md Developer Validations: `cargo clippy --all-targets -- -D warnings`, `cargo fmt --check`, and `cargo test` must all pass before committing. Also run `cargo run -- check`.

Keep a work log at `.waap/agents/${agent_id}/work_log.md`.

Before you start: `waap ticket update --ticket-id tt-add-agentsystemcodex-variant-and-cli-wiring --set-status in-progress`.

When validated, rebase your branch onto the latest `main` and `--ff-only` merge to `main`, resolving conflicts as needed. Include your agent id and the ticket id in your commit message.

IMPORTANT — do not skip: after your code is merged and validated, mark the ticket completed:
`waap ticket update --ticket-id tt-add-agentsystemcodex-variant-and-cli-wiring --set-status completed`
(waap marks your agent status completed automatically on a successful exit; you own the ticket status.)
