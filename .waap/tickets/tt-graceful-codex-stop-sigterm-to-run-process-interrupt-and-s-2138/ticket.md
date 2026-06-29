+++
title = "Graceful codex stop: SIGTERM-to-run-process interrupt and stop arm"
creation_date = 2026-06-29T19:07:21Z
status = "in-progress"
depends_on = ["tt-drive-codex-runs-runagentcodex-dispatch-and-turn-status-co-ce31"]
+++

# Goal

Implement `waap agent stop` for codex: because `turn/interrupt` needs the live JSON-RPC connection held only by the running `waap agent run` process (R), `stop` signals R, and R's SIGTERM handler issues a graceful `turn/interrupt` and closes the connection. Add the SIGTERM handler in the codex run path and the codex arm in `stop.rs`.

# Spec References

- `/specs/codex-agent-system.md` §5 "`waap agent stop`" (and the SIGTERM-handler sentence in §3).

# Current Implementation Context

- `src/agent/run.rs::run_agent_codex` (prior ticket) holds the live client + `thread_id` + `turn_id`. Per §3/§5 it must install a `SIGTERM` handler that calls `turn/interrupt(thread_id, turn_id)`, closes the connection, and returns through `run_in_agent_worktree` so the worktree is cleaned up. The interrupted turn yields a non-`Completed` `TurnStatus`, so `finalize_codex_run` leaves the agent `running` and does not overwrite the `aborted` status that `stop` writes to the record.
  - Implementation guidance: a blocking JSON-RPC pump cannot call back into the client from an async-signal context. Use a self-pipe / `signal_hook` flag / dedicated watcher so the `pump_until_turn_completed` loop observes the SIGTERM, invokes `turn_interrupt`, and unwinds. Pick the simplest approach that keeps `pump_until_turn_completed` testable; document the choice. Add `signal-handling` deps to `Cargo.toml` only if needed (prefer `signal-hook` if a crate is required).
- `src/agent/stop.rs`:
  - `stop_agents_with_systems` (lines ~41-55) builds an `abort(system, session_id)` closure; the `Claude` arm calls `kill_claude_session(session_id)` (`pkill -TERM -f <session_id>` in `src/claude.rs`), the `Opencode` arm calls `abort_opencode_session`.
  - `stop_agent_if_running` (lines ~77-97) currently only invokes `abort` when `session_id` is present and passes `(system, session_id)`. The codex arm needs the **agent id**, not the `session_id` (§5) — change the abort closure signature so it receives the `agent_id` (available in `stop_agent_if_running`) in addition to (or instead of) the session id. This is the one place codex diverges from the claude/opencode `abort(system, session_id)` shape; keep the opencode/claude arms working.
- Add a `signal_codex_run(agent_id)` helper (in `src/codex.rs` or alongside `kill_claude_session`) that runs `pkill -TERM -f "agent run --agent-id <agent-id>"`, mirroring `kill_claude_session`'s exit-code handling (0 or 1 ⇒ Ok). This matches R via its unique argv and NOT the `codex app-server --stdio` child (which lacks the agent id), independent of foreground/`nohup`/`setsid`.

# Required Behavior / Acceptance Criteria

1. `waap agent stop --agent-id <id>` for a running codex agent sends `SIGTERM` to R via `pkill -TERM -f "agent run --agent-id <id>"` and writes `aborted` to the record (the existing `stop_agent_if_running` already writes `aborted` after a successful abort).
2. R's SIGTERM handler issues `turn/interrupt(thread_id, turn_id)`, closes the connection, and returns; `run_in_agent_worktree` removes the worktree. The interrupted turn's non-`Completed` status means `finalize_codex_run` leaves the AGENT status alone (so `stop`'s `aborted` is not overwritten by a `completed`).
3. The codex arm uses the agent id, not the session id; opencode/claude stop behavior is unchanged.
4. Because a stdio app-server exits on stdin EOF, the child is torn down automatically if R dies for any reason; signalling R is the only stop path waap implements (no separate child-kill).

# Testing Expectations

- `stop_agents` tests inject the abort closure directly (see `agent_stop_kills_claude_process_instead_of_opencode_abort` and `agent_stop_aborts_opencode_sessions_for_running_agents`). Add a codex test asserting the codex arm is invoked with the **agent id** (not the session id) for a running `system = "codex"` agent, and that the record is marked `aborted` on success and left `running` when the abort closure errors (mirror `agent_stop_does_not_mark_aborted_when_claude_kill_fails`).
- After the signature change, update the existing opencode/claude stop tests to the new closure shape while preserving their assertions.
- Test `signal_codex_run`'s pkill exit-code mapping the way `kill_claude_session` is exercised (a `sh -c "exit N"`-style stand-in or asserting the 0/1-Ok, other-Err mapping). The SIGTERM-handler/`pump` interrupt path: unit-test the pump's "interrupt flag set ⇒ call turn_interrupt and return the resulting non-Completed status" behavior with an in-memory transport (no real signals/process).

# Dependency Rationale

Depends on `tt-drive-codex-runs-runagentcodex-dispatch-and-turn-status-co-ce31` because the SIGTERM handler lives inside `run_agent_codex` (needs the live client/thread_id/turn_id) and relies on `finalize_codex_run` leaving a non-`Completed` run `running` so `aborted` survives.

# Developer Validations (must pass before merge)

- `cargo clippy --all-targets -- -D warnings`
- `cargo fmt --check`
- `cargo test`
- `cargo run -- check`
