+++
creation_date = 2026-06-29T19:23:43Z
status = "ready"
+++

# Purpose
Implement code for `tt-drive-codex-runs-runagentcodex-dispatch-and-turn-status-co-ce31`.

# Instructions
Implement the functionality described in `/.waap/tickets/tt-drive-codex-runs-runagentcodex-dispatch-and-turn-status-co-ce31/ticket.md`, following the design in `/specs/codex-agent-system.md` (§3 and Completion). Use the JSON-RPC client already in `src/codex.rs`. Wire `run_agent_codex` into the `run_agent` dispatch in `src/agent/run.rs`, reusing `run_in_agent_worktree`/`mark_running`, and add `finalize_codex_run` for turn-status-based completion. Inspect the referenced source before implementing; make the smallest correct change.

`waap agent run` already prepares a git worktree for you and runs you inside it; do NOT create or remove a worktree yourself. Work in the current directory and commit on your branch.

Follow the AGENTS.md Developer Validations: `cargo clippy --all-targets -- -D warnings`, `cargo fmt --check`, and `cargo test` must all pass before committing. Also run `cargo run -- check`.

Keep a work log at `.waap/agents/${agent_id}/work_log.md`.

Before you start: `waap ticket update --ticket-id tt-drive-codex-runs-runagentcodex-dispatch-and-turn-status-co-ce31 --set-status in-progress`.

When validated, rebase your branch onto the latest `main` and `--ff-only` merge to `main`. Include your agent id and the ticket id in your commit message.

IMPORTANT — do not skip: after merge + validation, mark the ticket completed:
`waap ticket update --ticket-id tt-drive-codex-runs-runagentcodex-dispatch-and-turn-status-co-ce31 --set-status completed`
(waap marks your agent status completed automatically on a successful exit.)
