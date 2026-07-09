# Work Log

- Marked `tt-refactor-agent-worktree-lifecycle` in progress.
- Replaced callback lifecycle wrappers with an `AgentWorktree` guard that supports explicit cleanup, `Drop` cleanup, cleanup retries, and combined run/cleanup diagnostics.
- Made OpenCode, Claude, and Codex run flows sequential and passed `worktree_dir` explicitly to session, command, and app-server operations.
- Removed execution paths from backend configs and derived the OpenCode stop directory from `waap_root` plus the agent id.
- Replaced callback-oriented tests with guard, ordering, cleanup-error, backend-directory, and active-session tests.
- Updated lifecycle specifications to match the guard design.
