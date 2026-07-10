+++
name = "Reject duplicate agent session assignment"
creation_date = 2026-07-10T02:20:48Z
status = "abandoned"
+++

# Problem

This ticket has been folded into `.waap/tickets/tt-reject-already-running-agent-runs/ticket.md`, which now covers hidden no-op handling for already-running agents, duplicate session assignment, and already-completed finalization.

The original details are kept below for history.

---

`src/agent/run.rs` currently treats an agent session update as a no-op when the agent record already has the same `session_id` and `system`:

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

Like the already-`running` no-op in `mark_running(...)`, this can hide lifecycle mistakes. A session id should normally be assigned once per run. If it is already set to the exact value being written, that means the same session update path is being repeated, or an earlier partial run already persisted session state.

# Desired Behavior

Duplicate session assignment should not be silently accepted inside `update_agent_session(...)`.

Decide the intended invariant and enforce it explicitly:

- If `metadata.session_id` is already set, return an error explaining that the agent already has a session id.
- If `metadata.system` is already set to a different system, return an error explaining the system mismatch.
- If `metadata.system` already matches but `session_id` is empty/unset, it is still valid to write the new session id.

The error should happen before writing or committing any state.

# Context

The only plausible ways the session id/system would already match are:

- a retry after a previous run reached `update_agent_session(...)` and then crashed or was interrupted before the agent process completed
- a bug or duplicate call path invokes `update_agent_session(...)` twice for the same session
- manual state editing produced a pre-populated matching session id

Those cases should be visible as errors rather than reported as successful no-ops.

# Acceptance Criteria

- `update_agent_session(...)` no longer silently skips when `session_id` and `system` already match.
- Calling `update_agent_session(...)` for an agent that already has any `session_id` returns an error and does not commit.
- Calling `update_agent_session(...)` with a conflicting existing `system` returns an error and does not commit.
- Calling `update_agent_session(...)` with no existing session id and either no system or matching system still writes and commits the session id/system.
- Tests cover duplicate session id, conflicting system, and valid first assignment.
- Existing `mark_completed(...)` idempotency is preserved.
- Run the repository validation commands from `AGENTS.md` if feasible:

```sh
cargo clippy --all-targets -- -D warnings
cargo fmt --check
cargo build
cargo build --release
cargo test
```
