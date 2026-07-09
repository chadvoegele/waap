+++
name = "Refactor agent worktree cleanup"
creation_date = 2026-07-09T19:49:45Z
status = "pending"
+++

# Refactor Agent Worktree Cleanup

Replace the repeated `AgentWorktree::cleanup_on_error` calls in the agent runners with a closure-free, two-layer execution pattern.

## Requirements

- Add one cleanup boundary that accepts a completed `io::Result<T>`, always attempts worktree cleanup, and returns the combined result.
- Preserve the existing error semantics: a run error remains primary, a simultaneous cleanup error is included as diagnostics without changing the run error kind, and a cleanup error is returned when execution succeeded.
- Move each system's process or client lifecycle into a dedicated `run_*_in_worktree` function so ordinary `?` propagation can be used without skipping explicit cleanup.
- Apply the pattern consistently to OpenCode, Claude, and Codex runners.
- Ensure child process and Codex client handles are dropped before worktree cleanup begins.
- Keep `AgentWorktree`'s `Drop` implementation as a fallback.
- Update tests to cover successful execution with cleanup failure and simultaneous execution and cleanup failure.
- Run all validations required by `AGENTS.md`.

## Acceptance Criteria

- Agent runner code contains no repeated per-operation `cleanup_on_error` wrappers.
- Each runner has one explicit cleanup/result-combination boundary.
- Existing behavior and error diagnostics are preserved.
- Formatting, lint, debug and release builds, and tests pass.
