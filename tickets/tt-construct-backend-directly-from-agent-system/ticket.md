+++
name = "Construct backend directly from agent system"
creation_date = 2026-07-10T16:14:15Z
status = "completed"
+++

# Goal

Remove the backend resolver/registry abstraction and construct a backend directly from `AgentSystem`.

# Requirements

- Add one method on `AgentSystem` that lazily constructs and returns the selected backend, using `Box<dyn AgentSystemBackend>` or an equally simple owned representation.
- Preserve lazy OpenCode environment loading: Claude and Codex operations must not require OpenCode variables.
- Remove `BackendResolver`, `BackendRegistry`, their cached `Option` fields, `FakeResolver`, and resolver-specific tests.
- Refactor run orchestration so production selects the backend once and a focused helper accepts `&mut dyn AgentSystemBackend` for fake-backed tests.
- Refactor stop orchestration similarly. Resolve a production backend only after confirming an agent is running and has a session.
- Keep `stop --all` behavior correct when agents use different systems; repeated lightweight backend construction is acceptable.
- Preserve lifecycle transitions, session persistence, Git commits, reporting, worktree cleanup, start/wait behavior, abort behavior, and exit codes.
- Keep backend selection coverage and fake-backend orchestration coverage without introducing a replacement factory/resolver abstraction or closure.
- Avoid unrelated cleanup and compatibility layers.

# Acceptance Criteria

- Production backend selection is a single method associated with `AgentSystem`.
- No resolver/registry abstraction or backend cache remains.
- Run and stop orchestration tests inject a backend directly.
- Claude/Codex stop paths remain independent of OpenCode environment configuration.
- All validations pass: `cargo clippy --all-targets -- -D warnings`, `cargo fmt --check`, `cargo build`, `cargo build --release`, `cargo test`, `cargo run -- check`, and `waap check`.
