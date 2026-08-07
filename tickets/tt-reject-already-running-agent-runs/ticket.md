+++
name = "Reject hidden agent run lifecycle no-ops"
creation_date = 2026-07-10T02:19:48Z
status = "completed"
+++

# Problem

`src/agent/run.rs` currently treats several unexpected agent lifecycle states as successful no-ops.

`mark_running(...)` treats an already-`running` agent as success:

```rust
let (current, _) = read_agent_record(waap_root, agent_id)?;
if current.status == "running" {
    let report = load_agent_report(waap_root, agent_id)?;
    print_run_agent_report(output_format, "Running agent", &report, "");
    return Ok(());
}
```

This makes `waap agent run --agent-id <id>` appear successful even though no new agent process is launched. That can hide operator mistakes and makes the run lifecycle unclear.

`update_agent_session(...)` also treats an already-matching `session_id`/`system` as success:

```rust
let header = format!("{} session", system.as_str());
if metadata.session_id.as_deref() == Some(session_id)
    && metadata.system.as_ref() == Some(&system)
{
    let report = load_agent_report(waap_root, agent_id)?;
    print_run_agent_report(output_format, &header, &report, "");
    return Ok(());
}
```

A session id should normally be assigned once per run. If it is already set to the exact value being written, the likely explanations are a retry after partial run state was persisted, a duplicate call path, or manual state editing. Those should be visible rather than reported as successful no-ops.

`mark_completed(...)` treats an already-`completed` agent as success:

```rust
if metadata.status == "completed" {
    let report = load_agent_report(waap_root, agent_id)?;
    print_run_agent_report(output_format, "Completed agent", &report, "");
    return Ok(());
}
```

This may have been intended to support a system/agent self-marking completed before the runner finalizes, but it should be reviewed against the desired invariant. If completion is runner-owned, an already-`completed` record should probably be treated as an unexpected lifecycle state rather than silently accepted.

# Desired Behavior

Trying to run an agent whose current status is already `running` should fail before any system-specific setup occurs.

Move the guard higher than `mark_running(...)`, ideally into the top-level `run_agent(...)` path before dispatching to Opencode, Claude, or Codex.

`mark_running(...)` should be responsible only for marking and committing the transition to `running`; it should not silently accept already-running agents.

Duplicate session assignment should not be silently accepted inside `update_agent_session(...)`:

- If `metadata.session_id` is already set, return an error explaining that the agent already has a session id.
- If `metadata.system` is already set to a different system, return an error explaining the system mismatch.
- If `metadata.system` already matches but `session_id` is empty/unset, it is still valid to write the new session id.

Review `mark_completed(...)` and decide the intended invariant explicitly:

- If runner-owned completion is the invariant, remove the already-`completed` no-op and return an error when completion has already happened unexpectedly.
- If agent self-completion is intentionally supported, keep the no-op but document the concrete path and test it directly so the behavior is intentional rather than incidental.

# Acceptance Criteria

- `run_agent(...)` reads the current agent record before dispatching to a system-specific runner.
- If the current status is `running`, `run_agent(...)` returns an error such as `agent <id> is already running`.
- The guard runs before system-specific config/session setup and before `mark_running(...)`.
- `mark_running(...)` no longer contains the already-running no-op/report branch.
- `update_agent_session(...)` no longer silently skips when `session_id` and `system` already match.
- Calling `update_agent_session(...)` for an agent that already has any `session_id` returns an error and does not commit.
- Calling `update_agent_session(...)` with a conflicting existing `system` returns an error and does not commit.
- Calling `update_agent_session(...)` with no existing session id and either no system or matching system still writes and commits the session id/system.
- `mark_completed(...)` already-`completed` behavior is made explicit: either rejected as an unexpected lifecycle state, or retained with a documented and tested reason.
- Add or update tests proving an already-running agent cannot be run and that the system-specific runner is not reached.
- Tests cover duplicate session id, conflicting system, valid first session assignment, and the chosen already-`completed` behavior.
- Run the repository validation commands from `AGENTS.md` if feasible:

```sh
cargo clippy --all-targets -- -D warnings
cargo fmt --check
cargo build
cargo build --release
cargo test
```
