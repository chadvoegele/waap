+++
creation_date = 2026-06-29T19:13:48Z
status = "ready"
+++

# Purpose
Implement code for `tt-implement-codex-app-server-json-rpc-client-srccodexrs`.

# Instructions
Implement the functionality described in `/.waap/tickets/tt-implement-codex-app-server-json-rpc-client-srccodexrs/ticket.md`, following the design in `/specs/codex-agent-system.md`. For exact codex JSON-RPC method/param/field names, study `/home/cvoegele/code/github.com/openai/codex` (`codex-rs/app-server/README.md`, `app-server-protocol/`); you can also run `codex app-server generate-json-schema` if codex is installed. Make the smallest correct change.

`waap agent run` already prepares a git worktree for you and runs you inside it; do NOT create or remove a worktree yourself. Work in the current directory and commit on your branch.

Follow the AGENTS.md Developer Validations: `cargo clippy --all-targets -- -D warnings`, `cargo fmt --check`, and `cargo test` must all pass before committing. Also run `cargo run -- check`.

Keep a work log at `.waap/agents/${agent_id}/work_log.md`.

Before you start: `waap ticket update --ticket-id tt-implement-codex-app-server-json-rpc-client-srccodexrs --set-status in-progress`.

When validated, rebase your branch onto the latest `main` and `--ff-only` merge to `main`. Include your agent id and the ticket id in your commit message.

IMPORTANT — do not skip: after merge + validation, mark the ticket completed:
`waap ticket update --ticket-id tt-implement-codex-app-server-json-rpc-client-srccodexrs --set-status completed`
(waap marks your agent status completed automatically on a successful exit.)
