+++
creation_date = 2026-06-29T15:46:26Z
status = "completed"
session_id = "333f3044-0e23-4040-a149-0203ab7f4799"
system = "claude"
+++

# Purpose
Produce the implementation plan described in `/.waap/tickets/tt-plan-add-codex-as-an-agent-run-system/ticket.md`. This is a PLANNING task: study the source and write a plan document. Do NOT implement the codex system change.

# Instructions
Read `/.waap/tickets/tt-plan-add-codex-as-an-agent-run-system/ticket.md` and study both source trees it references:
- The codex source at `/home/cvoegele/code/github.com/openai/codex` (especially `codex-rs/exec/src/cli.rs` and `docs/exec.md`) to confirm the exact `codex exec` flags, JSON output, and session/resume behavior.
- The waap source (`src/agent.rs`, `src/cli.rs`, `src/claude.rs`, `src/opencode.rs`, `src/agent/run.rs`, `src/agent/stop.rs`) to mirror the existing system wiring.

Write the plan to `specs/codex-agent-system.md` covering all nine points and the acceptance criteria in the ticket, with concrete codex flags and concrete waap function/file references. Resolve the session-id acquisition strategy with a clear recommendation and rationale. Do not change any `src/` behavior.

`waap agent run` already prepares a git worktree for you and runs you inside it; do NOT create or remove a worktree yourself. Make your changes in the current working directory and commit on your branch.

Before you start: `waap ticket update --ticket-id tt-plan-add-codex-as-an-agent-run-system --set-status in-progress`.

Validate per the ticket: `cargo fmt --check`, `cargo test`, and `cargo run -- check` must still pass (the doc-only change should not affect them). Then rebase your branch onto the latest `main` and `--ff-only` merge to `main`. Include your agent id and the ticket id in your commit message.

IMPORTANT — do not skip: after the plan is merged, you MUST mark the ticket completed:
`waap ticket update --ticket-id tt-plan-add-codex-as-an-agent-run-system --set-status completed`
(waap marks your agent status completed automatically on a successful exit; you own the ticket status.)
