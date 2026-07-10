+++
name = "Plan agent system backend trait"
creation_date = 2026-07-10T14:04:59Z
status = "in-progress"
depends_on = ["tt-refactor-agent-run-cleanup-and-completion", "tt-reject-already-running-agent-runs", "tt-trim-agent-run-comments", "tt-simplify-codex-json-rpc-client"]
+++

# Goal

Review the agent-system architecture after the currently pending run and Codex refactors land, and produce a concrete design for consolidating system-specific run and abort behavior.

# Context

`src/agent/run.rs` dispatches `AgentSystem::Opencode`, `AgentSystem::Claude`, and `AgentSystem::Codex` to separate run functions. `src/agent/stop.rs` repeats system dispatch through an injected closure for abort behavior. This scatters the behavioral definition of an agent system and gives tests a low-level callback seam rather than an explicit system abstraction.

The preceding tickets intentionally change run lifecycle structure, lifecycle invariants, comments, and the Codex client. Review the resulting code rather than designing against the pre-refactor implementation.

# Deliverable

Create a reviewable design at `specs/agent-system-backend.md`. Do not refactor production Rust code in this ticket.

# Design Questions

- Whether to retain `AgentSystem` as the persisted/CLI enum and introduce a separate behavioral trait, or use another idiomatic Rust representation.
- The interface for system-specific `run` and `abort` behavior.
- Where Opencode, Claude, and Codex configuration is loaded and retained, including lazy Opencode configuration for stop operations.
- How common run lifecycle concerns remain shared: record transitions, commits/reporting, worktree creation and cleanup, session persistence, completion, and error propagation.
- How system-specific differences remain explicit, especially session creation timing and Codex interruption through the live `waap agent run` process.
- How enum-to-implementation construction and dispatch occur in one well-defined place.
- How tests inject fake system behavior without HTTP requests or process signals, and whether static dispatch, trait objects, an enum wrapper, or a factory is appropriate.
- A migration sequence that avoids unnecessary abstractions and minimizes churn.

# Acceptance Criteria

- The plan is based on the code after all blocking tickets are completed.
- The plan specifies concrete trait/type signatures and ownership/lifetime choices.
- The plan maps the current Opencode, Claude, and Codex run and abort paths to the proposed design.
- Shared orchestration remains separate from backend-specific behavior without forcing materially different systems into an artificial common flow.
- Test strategy and dependency-injection seams are explicit.
- Risks, rejected alternatives, affected files, and an implementation/validation checklist are included.
- No production behavior changes are made.
- `cargo run -- check`, `cargo fmt --check`, and `cargo test` pass.
