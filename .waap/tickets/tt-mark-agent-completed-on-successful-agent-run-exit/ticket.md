+++
title = "Mark agent completed on successful agent run exit"
creation_date = 2026-06-29T15:04:11Z
status = "pending"
depends_on = ["tt-linearize-agent-run-git-history-with-rebase-and-fast-forwa-d778"]
+++

# Problem

Agents frequently exit without running the final `waap agent update --set-status completed` step, leaving agents stuck in `running` even though their system process finished successfully and merged its work. The completion signal is model-driven and unreliable.

# Desired Behavior

`waap agent run` should derive the agent's terminal status from the system process instead of relying on the agent to self-report.

- After the attached system process exits, if it exited with status code 0, `waap agent run` sets the agent's `status = "completed"` and commits that state change on `main`. This happens after the agent has merged its branch, so the completion lands cleanly on `main`.
- On a non-zero exit, the agent is NOT marked completed. Leave it `running` (or introduce a distinct failed/error status) so the failure is visible. Decide and document the chosen behavior.
- This sets only the AGENT status. `waap` does not mark the ticket completed, because the agent record carries no ticket reference; ticket completion remains the agent's responsibility.
- Remove the instruction telling agents to mark their own status completed (the agent still marks its ticket status).

# Notes / Future Work

Exit code 0 is a coarse proxy for "goal completed". The `claude --output-format json` result already includes `subtype`, `is_error`, and `terminal_reason` fields; a future ticket may parse a richer goal-completed signal. Out of scope here.

# Acceptance Criteria

1. A successful (exit 0) `waap agent run` leaves the agent `status = "completed"` on `main` without the agent running `waap agent update`.
2. A non-zero exit does not mark the agent completed; the documented failure behavior is applied.
3. The completion commit lands on `main` after the agent's merge (visible in main worktree, history stays linear per the dependency ticket).
4. Agent role/instruction docs no longer ask the agent to self-mark its status completed; ticket-status responsibility is unchanged.
5. Tests cover completed-on-zero-exit, not-completed-on-nonzero-exit, and that no ticket status is changed by waap.

# Validation

- `cargo clippy --all-targets -- -D warnings`
- `cargo fmt --check`
- `cargo test`
- `cargo run -- check`
