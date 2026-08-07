# Work Log

- Read the ticket, developer role, backend design, orchestration, backend implementations, and tests. Marked the ticket in progress.
- Replaced preparation and session callbacks with `AgentSystemBackend::start`, `StartedRun`, and the object-safe owned `RunHandle::wait` interface.
- Kept lifecycle persistence in shared orchestration: mark running, create the worktree, start the backend, commit its returned session ID, wait, then clean up and process the outcome.
- Moved OpenCode session creation and process spawn, Claude UUID generation and process spawn, and Codex app-server initialization and thread creation into each backend's `start`. Codex's run handle retains its client, interrupt flag, thread ID, and prompt.
- Updated fake backend tests for system selection, start context and worktree ordering, session commit ordering, successful and failed wait outcomes, and start/wait errors.
- Verified `cargo clippy --all-targets -- -D warnings`, `cargo fmt --check`, `cargo build`, `cargo build --release`, `cargo test`, `cargo run -- check`, and `waap check` pass.
