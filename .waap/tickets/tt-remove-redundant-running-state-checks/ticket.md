+++
name = "Remove redundant running state checks"
creation_date = 2026-07-10T16:39:48Z
status = "in-progress"
+++

# Goal

Remove redundant non-atomic running-state checks while preserving early rejection before backend construction.

# Requirements

- Keep `reject_running_agent()` in the top-level `run_agent()` before `AgentSystem::backend()`.
- Remove the duplicate running check from `run_agent_with_backend()`.
- Remove the re-read/running check and misleading concurrency comment from `mark_running()`.
- Do not add locks or claim atomic concurrent-start protection in this ticket.
- Update tests that directly call `run_agent_with_backend()` or `mark_running()` so they test each helper’s actual responsibility.
- Preserve all other lifecycle behavior, state commits, reporting, backend selection, worktree handling, and exit codes.
- Avoid unrelated cleanup.

# Acceptance Criteria

- Production performs one running-state precondition check before backend construction.
- `mark_running()` only performs the running transition, write, commit, and report.
- No test or comment claims repeated reads prevent concurrent starts.
- All validations pass: `cargo clippy --all-targets -- -D warnings`, `cargo fmt --check`, `cargo build`, `cargo build --release`, `cargo test`, `cargo run -- check`, and `waap check`.
