# Work Log

- Marked `tt-refactor-agent-worktree-lifecycle` in progress.
- Replaced callback-based worktree runners with an `AgentWorktree` guard that supports explicit cleanup, drop cleanup, and deterministic run/cleanup error precedence.
- Removed execution paths from OpenCode, Claude, and Codex configs; system APIs now receive `worktree_dir` explicitly.
- Reworked lifecycle tests around the guard and added focused directory, branch-base, session-visibility, and cleanup-error coverage.
- Passed `cargo clippy --all-targets -- -D warnings`, `cargo fmt --check`, `cargo build`, `cargo build --release`, `cargo test` (228 unit, 23 integration), and `waap check`.
