+++
name = "Replace session publisher with started run"
creation_date = 2026-07-10T15:48:51Z
status = "pending"
+++

# Goal

Replace the session-publishing closure and preparation split with a two-phase backend start/wait interface.

# Requirements

- Remove `RunPreparation`, `RunContext::initial_session_id`, and `RunContext::publish_session`.
- Introduce a backend start operation that receives the worktree and returns the discovered/generated session id plus an owned run handle.
- Introduce an object-safe owned run handle whose wait operation returns `RunOutcome`.
- Shared orchestration must mark the agent running, create the worktree, call backend start, persist the returned session id through `update_agent_session`, then wait for completion and clean up.
- OpenCode must create its server session and spawn its process in start.
- Codex must initialize the app server and create its thread in start, retaining client and interrupt state in its run handle; turn execution may occur in wait as appropriate.
- Claude must generate its UUID and spawn its process in start.
- Keep lifecycle persistence, Git commits, error propagation, worktree cleanup, stop behavior, exit codes, and backend laziness unchanged.
- Backends must not directly update WAAP metadata or perform WAAP Git commits.
- Update fake backend coverage for backend selection, start context, session publication, and wait outcomes.
- Avoid compatibility layers and unrelated cleanup.

# Acceptance Criteria

- No session publisher closure or callback remains.
- All three systems return their session id from the same backend start interface.
- Session metadata is persisted by shared orchestration before waiting for the run to complete.
- OpenCode and Codex still create sessions only after the worktree exists.
- Full validation passes: `cargo clippy --all-targets -- -D warnings`, `cargo fmt --check`, `cargo build`, `cargo build --release`, `cargo test`, `cargo run -- check`, and `waap check`.
