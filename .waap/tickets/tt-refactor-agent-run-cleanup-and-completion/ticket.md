+++
name = "Refactor agent run cleanup and completion"
creation_date = 2026-07-10T02:16:33Z
status = "pending"
+++

# Problem

`src/agent/run.rs` currently hides run/cleanup error handling behind `AgentWorktree::finish(...)`, which makes the worktree type responsible for combining process-run results with cleanup results. The call sites are harder to read because cleanup is implicit.

The completion path also goes through `finalize_agent_run(...)` and `finalize_run(...)`, even though the desired behavior is simple: if the system run succeeded, call `mark_completed(...)`; otherwise leave the agent `running` so the failure is visible. This should behave similarly to `mark_running(...)` and avoid unnecessary layers.

# Desired Refactor

Replace the `AgentWorktree::finish(...)` pattern with explicit run and cleanup results at each run call site:

```rust
let run_result = run_opencode_in_worktree(...);
let cleanup_result = worktree.cleanup();
let status = collapse_errors(run_result, cleanup_result)?;
```

Add a private helper named `collapse_errors` that preserves the current behavior:

- return the run value when both run and cleanup succeed
- return the cleanup error when run succeeds but cleanup fails
- return the run error when run fails but cleanup succeeds
- return the run error with cleanup diagnostics appended when both fail

Remove `AgentWorktree::finish(...)`; keep `Drop` as a best-effort cleanup safety net for early exits.

Replace `finalize_agent_run(...)`/`finalize_run(...)` layering for non-Codex systems with direct completion logic at the call sites:

- if `ExitStatus::success()` is true, call `mark_completed(waap_root, output_format, agent_id)?`
- return the CLI exit code derived from the process status
- on non-zero exit, do not mark completed and do not commit

Apply the same direct style to Codex using `TurnStatus::is_success()`.

# Acceptance Criteria

- `run_agent_opencode`, `run_agent_claude`, and `run_agent_codex` explicitly call `worktree.cleanup()` and `collapse_errors(...)`.
- `AgentWorktree::finish(...)` is removed.
- `collapse_errors` is covered by tests for cleanup failure after success and both-run-and-cleanup failure.
- Successful Opencode/Claude-style process exits call `mark_completed(...)` directly.
- Failed process exits leave the agent `running` and return the process exit code.
- Successful Codex turns call `mark_completed(...)` directly; non-success turns leave the agent `running` and return failure.
- Existing behavior around commit messages, no redundant completed commits, and not touching ticket status is preserved.
- Run the repository validation commands from `AGENTS.md` if feasible:

```sh
cargo clippy --all-targets -- -D warnings
cargo fmt --check
cargo build
cargo build --release
cargo test
```
