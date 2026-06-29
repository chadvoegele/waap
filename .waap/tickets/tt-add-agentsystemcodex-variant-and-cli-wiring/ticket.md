+++
title = "Add AgentSystem::Codex variant and CLI wiring"
creation_date = 2026-06-29T19:05:14Z
status = "in-progress"
+++

# Goal

Add `Codex` as a third `AgentSystem` variant so frontmatter `system = "codex"` validates and `waap agent run --system codex` parses. This is the foundation ticket for the codex agent-run system (spec `/specs/codex-agent-system.md` §1).

# Spec References

- `/specs/codex-agent-system.md` §1 "`AgentSystem::Codex` variant and CLI wiring".

# Current Implementation Context

- `src/agent.rs`:
  - `enum AgentSystem { #[default] Opencode, Claude }` (around line 195).
  - `AgentSystem::as_str` matches each variant to its label (`"opencode"`, `"claude"`).
  - `parse` and `labels` iterate `AgentSystem::value_variants()`, so they need **no** change — adding a variant makes `system = "codex"` validate (via `require_optional_string_choice(... &AgentSystem::labels() ...)` in `AgentMetadata::from_frontmatter`) and `--system codex` parse automatically.
- `src/cli.rs`: `--system` on `AgentCommand::Run` is `#[arg(long, value_enum, default_value = "opencode")]` over `AgentSystem`. No structural change. The default stays `opencode`.

# Required Behavior / Acceptance Criteria

1. `AgentSystem` gains a `Codex` variant; `as_str` returns `"codex"` for it. Do not change `#[default]` (stays `Opencode`).
2. `AgentSystem::parse("codex")` returns `Some(AgentSystem::Codex)`; `AgentSystem::labels()` includes `"codex"`.
3. Agent frontmatter `system = "codex"` passes `waap check` / `AgentMetadata::from_frontmatter`.
4. `waap agent run --agent-id <id> --system codex` parses to `AgentSystem::Codex`. Note: `run_agent` dispatch is NOT wired here (a later ticket adds `run_agent_codex`); to keep this ticket compilable and self-contained, add a `AgentSystem::Codex` arm to the `run_agent` match in `src/agent/run.rs` that returns a clear "not yet implemented" `io::Error` (it will be replaced by the run ticket). Document this stub in a code comment referencing the dependent ticket.

# Testing Expectations

- In `src/cli.rs`, the existing test `agent_run_rejects_invalid_system_argument` (lines ~432-446) uses `"codex"` as the rejected value — `codex` is now VALID, so this test must change. Replace it with: (a) a positive test that `--system codex` parses to `AgentSystem::Codex`, and (b) an invalid-value test using a still-unknown system string (e.g. `"cursor"`).
- Add/extend a unit test in `src/agent.rs` asserting `AgentSystem::parse("codex") == Some(Codex)`, `as_str() == "codex"`, and that `labels()` contains `"codex"`.
- Add an `AgentMetadata::from_frontmatter` test (mirroring `agent_metadata_known_fields_pass`) with `system = "codex"` passing.

# Developer Validations (must pass before merge)

- `cargo clippy --all-targets -- -D warnings`
- `cargo fmt --check`
- `cargo test`
- `cargo run -- check`
