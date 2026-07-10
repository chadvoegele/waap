# Work Log

- Read the ticket, developer workflow, and existing agent run lifecycle implementation and tests.
- Marked `tt-reject-already-running-agent-runs` in progress.
- Chose runner-owned completion because the existing finalization path explicitly derives completion from the system result; duplicate completion will be rejected.
- Added the pre-dispatch already-running guard and a race-safe rejection in `mark_running`.
- Made session assignment reject any existing session id or conflicting system while preserving first assignment for unset or matching systems.
- Made runner-owned completion reject an already-completed record.
- Added tests for all requested lifecycle paths, including assertions that rejected operations do not commit or create a worktree.
- Passed `cargo clippy --all-targets -- -D warnings`, `cargo fmt --check`, `cargo build`, `cargo build --release`, and `cargo test` (230 unit, 8 root/validation, and 13 state-commit tests).
