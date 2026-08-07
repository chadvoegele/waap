+++
name = "Enforce agent lifecycle state graph"
creation_date = 2026-07-10T18:50:12Z
status = "completed"
+++

# Goal

Introduce and enforce an explicit agent lifecycle state graph.

# State Graph

- `ready -> running`
- `ready -> aborted`
- `running -> completed`
- `running -> failed`
- `running -> aborted`
- `completed`, `failed`, and `aborted` are terminal.

# Requirements

- Add `AgentStatus::Failed` and support it consistently in CLI parsing, filtering, reports, metadata validation, and tests.
- Centralize transition validation instead of scattering string comparisons.
- Starting an agent must require `ready -> running`.
- Successful completion must require `running -> completed`; it must never overwrite `aborted` or another terminal state.
- Nonzero `RunOutcome` must persist `running -> failed` before returning its exit code.
- Errors after the running transition, including worktree creation, backend start, session persistence, wait, and cleanup errors, must attempt `running -> failed` while preserving the primary error if failure-state persistence also fails.
- Backend construction errors before the running transition leave the agent `ready`.
- Stop must enforce `running -> aborted`; permit `ready -> aborted` for explicit cancellation where the command/API supports it.
- Apply transition validation to `agent update --set-status`; invalid, same-state, and terminal-state transitions must be rejected.
- Session assignment must require the agent to remain `running` and have no existing session.
- Retrying terminal agents is out of scope; users create a new agent.
- Do not add locking or claim atomic cross-process enforcement.
- Preserve backend behavior, Git/reporting conventions, worktree cleanup, and ticket state.
- Avoid unrelated cleanup and compatibility layers.

# Tests

- Add a table-driven test for every allowed and rejected transition.
- Cover successful completion, nonzero failure, post-running errors, stop/cancel, terminal rerun rejection, aborted-not-overwritten-by-completion, and update command enforcement.
- Existing lifecycle and integration coverage must continue passing.

# Validation

- `cargo clippy --all-targets -- -D warnings`
- `cargo fmt --check`
- `cargo build`
- `cargo build --release`
- `cargo test`
- `cargo run -- check`
- `waap check`
