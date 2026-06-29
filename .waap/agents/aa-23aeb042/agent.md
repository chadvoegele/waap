+++
creation_date = 2026-06-29T19:29:59Z
status = "running"
session_id = "2192c8ca-0dca-4f76-b4f7-e104b4d8a413"
system = "claude"
+++

# Purpose
Implement code for `tt-graceful-codex-stop-sigterm-to-run-process-interrupt-and-s-2138`.

# Instructions
Implement the functionality described in `/.waap/tickets/tt-graceful-codex-stop-sigterm-to-run-process-interrupt-and-s-2138/ticket.md`, following the design in `/specs/codex-agent-system.md` (§5). Add the SIGTERM handler in the codex run path that calls `turn/interrupt` and closes the connection, and add the `AgentSystem::Codex` arm in `src/agent/stop.rs::stop_agents_with_systems` (keyed on agent_id, not session_id — adjust the abort closure as the spec describes). Inspect the referenced source before implementing; make the smallest correct change.

`waap agent run` already prepares a git worktree for you and runs you inside it; do NOT create or remove a worktree yourself. Work in the current directory and commit on your branch.

Follow the AGENTS.md Developer Validations: `cargo clippy --all-targets -- -D warnings`, `cargo fmt --check`, and `cargo test` must all pass before committing. Also run `cargo run -- check`.

Keep a work log at `.waap/agents/${agent_id}/work_log.md`.

Before you start: `waap ticket update --ticket-id tt-graceful-codex-stop-sigterm-to-run-process-interrupt-and-s-2138 --set-status in-progress`.

When validated, rebase your branch onto the latest `main` and `--ff-only` merge to `main`. Include your agent id and the ticket id in your commit message.

IMPORTANT — do not skip: after merge + validation, mark the ticket completed:
`waap ticket update --ticket-id tt-graceful-codex-stop-sigterm-to-run-process-interrupt-and-s-2138 --set-status completed`
(waap marks your agent status completed automatically on a successful exit.)
