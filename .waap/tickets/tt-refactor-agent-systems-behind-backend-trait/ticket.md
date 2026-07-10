+++
name = "Refactor agent systems behind backend trait"
creation_date = 2026-07-10T14:05:18Z
status = "completed"
depends_on = ["tt-plan-agent-system-backend-trait"]
+++

# Goal

Implement the reviewed agent-system abstraction designed by `tt-plan-agent-system-backend-trait` and documented in `specs/agent-system-backend.md`.

# Context

Agent-system behavior is currently dispatched separately in the run and stop paths. The planning ticket will review the code after the preceding lifecycle and Codex refactors and define the exact abstraction, ownership model, construction boundary, and test seams.

# Requirements

- Follow `specs/agent-system-backend.md`; do not substitute the preliminary ideas in this ticket for the reviewed design.
- Consolidate Opencode, Claude, and Codex run and abort behavior behind the planned agent-system abstraction.
- Remove duplicated system dispatch and the callback closure used by the production stop path where the plan calls for it.
- Preserve the persisted and CLI representation of agent systems unless the plan explicitly justifies a compatible change.
- Preserve run lifecycle behavior, worktree cleanup, record transitions, commits, reporting, exit codes, session persistence, and abort semantics.
- Keep Opencode configuration loading appropriately lazy so stopping Claude or Codex agents does not require Opencode environment variables.
- Keep Codex abort behavior keyed by `agent_id` and its live run-process interrupt flow unless the reviewed plan identifies a tested correction.
- Add or update test implementations of the abstraction so run and abort orchestration can be tested without real HTTP calls, agent processes, or process signals.
- Avoid unrelated cleanup and compatibility layers.

# Acceptance Criteria

- The implementation matches the types, responsibilities, and migration strategy in `specs/agent-system-backend.md`.
- System-specific run and abort behavior has one clear ownership location per backend.
- Top-level run and stop orchestration no longer maintain independent `AgentSystem` dispatch logic where the design intends backend polymorphism.
- Tests cover backend selection and prove that each system receives the correct agent id, session id, worktree, and lifecycle context.
- Existing Opencode, Claude, and Codex behavior remains covered.
- All developer validations pass from the repository root:

```sh
cargo clippy --all-targets -- -D warnings
cargo fmt --check
cargo build
cargo build --release
cargo test
cargo run -- check
```
