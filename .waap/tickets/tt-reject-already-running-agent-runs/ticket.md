+++
name = "Reject already running agent runs"
creation_date = 2026-07-10T02:19:48Z
status = "pending"
+++

# Problem

`src/agent/run.rs` currently treats an already-`running` agent as a no-op inside `mark_running(...)`:

```rust
let (current, _) = read_agent_record(waap_root, agent_id)?;
if current.status == "running" {
    let report = load_agent_report(waap_root, agent_id)?;
    print_run_agent_report(output_format, "Running agent", &report, "");
    return Ok(());
}
```

This makes `waap agent run --agent-id <id>` appear successful even though no new agent process is launched. That can hide operator mistakes and makes the run lifecycle unclear.

# Desired Behavior

Trying to run an agent whose current status is already `running` should fail before any system-specific setup occurs.

Move the guard higher than `mark_running(...)`, ideally into the top-level `run_agent(...)` path before dispatching to Opencode, Claude, or Codex.

`mark_running(...)` should be responsible only for marking and committing the transition to `running`; it should not silently accept already-running agents.

# Acceptance Criteria

- `run_agent(...)` reads the current agent record before dispatching to a system-specific runner.
- If the current status is `running`, `run_agent(...)` returns an error such as `agent <id> is already running`.
- The guard runs before system-specific config/session setup and before `mark_running(...)`.
- `mark_running(...)` no longer contains the already-running no-op/report branch.
- Existing `mark_completed(...)` idempotency is preserved.
- Existing `update_agent_session(...)` idempotency is preserved.
- Add or update tests proving an already-running agent cannot be run and that the system-specific runner is not reached.
- Run the repository validation commands from `AGENTS.md` if feasible:

```sh
cargo clippy --all-targets -- -D warnings
cargo fmt --check
cargo build
cargo build --release
cargo test
```
