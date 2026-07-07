+++
name = "Move agent system adapters under src/agent"
creation_date = 2026-07-07T10:50:29Z
status = "in-progress"
+++

# Goal

Move the agent-system adapter modules `src/opencode.rs`, `src/claude.rs`, and `src/codex.rs` under the existing `src/agent/` module tree.

# Background

Current usage indicates these modules are only consumed by agent run/stop code:

- `src/main.rs` declares the top-level modules.
- `src/agent/run.rs` imports Claude, OpenCode, and Codex run/config/client helpers.
- `src/agent/stop.rs` imports Claude/OpenCode/Codex stop helpers.
- Tests for these helpers are colocated in the same module files.

That makes these files agent implementation details rather than top-level application modules.

# Desired Behavior

- Move:
  - `src/opencode.rs` to `src/agent/opencode.rs`
  - `src/claude.rs` to `src/agent/claude.rs`
  - `src/codex.rs` to `src/agent/codex.rs`
- Expose them through the `agent` module tree rather than top-level `mod` declarations in `src/main.rs`.
- Update imports in `src/agent/run.rs`, `src/agent/stop.rs`, and any tests to use the new module paths.
- Keep visibility no broader than necessary; prefer `pub(super)` / `pub(crate)` consistently with the surrounding agent module code.
- Preserve all behavior and test coverage. This is a file/module organization refactor only.
- Update any current documentation/spec references that point at the moved source paths if they are still intended to be live implementation references.

# Acceptance Criteria

- `src/main.rs` no longer declares `mod opencode;`, `mod claude;`, or `mod codex;` as top-level modules.
- The moved modules are declared from the `src/agent` module tree.
- `agent run` and `agent stop` continue to compile and use the same adapter helpers.
- Existing adapter tests still run from their new module locations.
- No behavior changes to OpenCode, Claude, or Codex run/stop flows.
- Developer validations pass:
  - `cargo clippy --all-targets -- -D warnings`
  - `cargo fmt --check`
  - `cargo build`
  - `cargo build --release`
  - `cargo test`
