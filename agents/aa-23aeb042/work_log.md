# Work log — aa-23aeb042

Ticket: `tt-graceful-codex-stop-sigterm-to-run-process-interrupt-and-s-2138`
Spec: `/specs/codex-agent-system.md` §3 (SIGTERM handler) and §5 (`waap agent stop`).

## Goal

Implement graceful `waap agent stop` for codex: `stop` signals the running
`waap agent run` process (R) with SIGTERM, R's handler issues a graceful
`turn/interrupt` and closes the connection, leaving the interrupted turn
non-`Completed` so `finalize_codex_run` does not overwrite the `aborted` status.

## Changes

### `src/codex.rs`
- Removed the module-wide `#![allow(dead_code)]` (it existed only because
  `turn_interrupt` was unused; it is now wired into the stop path). Handled the
  remaining pre-existing dead items individually:
  - `completed_status_for_turn` now checks `method == "turn/completed"` (uses the
    previously-unused `METHOD_TURN_COMPLETED`; also more precise).
  - `CodexClient::child` annotated `#[allow(dead_code)]` (RAII guard, never read).
  - `CodexClient::new` annotated `#[cfg(test)]` (only used by tests).
- Added `signal_codex_run(agent_id)`: runs `pkill -TERM -f "agent run --agent-id
  <id>"`, mirroring `kill_claude_session`'s exit-code handling (0/1 ⇒ Ok). The
  exit-code mapping was factored into `map_pkill_status(command)` so it is
  unit-testable with `sh -c "exit N"` stand-ins.
- `pump_until_turn_completed` now takes an `interrupt: &AtomicBool`. At the top
  of each loop iteration it observes the flag once, issues a single
  `turn/interrupt`, and returns the resulting non-`Completed` status from the
  server's `turn/completed`.

### `src/agent/run.rs`
- `run_agent_codex` installs a SIGTERM handler via
  `signal_hook::flag::register(SIGTERM, Arc<AtomicBool>)` and passes the flag to
  `pump_until_turn_completed`.

### `src/agent/stop.rs`
- Abort closure signature changed from `(system, session_id)` to
  `(system, agent_id, session_id)`. claude/opencode still key on `session_id`;
  the new `Codex` arm calls `signal_codex_run(agent_id)`.

### `Cargo.toml`
- Added `signal-hook` for the SIGTERM handler.

## Design choice: flag observed at loop top (no read wake-up)

A blocking JSON-RPC read cannot call back into the client from an async-signal
context, and signal-hook's handler does not interrupt the blocking read. The
pump therefore checks the `AtomicBool` at the top of each loop iteration: during
an active turn codex streams notifications continuously, so the interrupt is
acted on as soon as the next inbound message unblocks the read. This is the
simplest approach that keeps `pump_until_turn_completed` unit-testable with an
in-memory transport (pre-set the flag, no real signals/process).

## Tests added
- `pump_interrupts_when_flag_is_set_and_returns_interrupted_status`
- `pump_does_not_interrupt_when_flag_is_unset`
- `signal_status_maps_zero_and_one_to_ok_and_other_to_err`
- `agent_stop_signals_codex_run_with_agent_id_not_session_id`
- `agent_stop_does_not_mark_aborted_when_codex_signal_fails`
- Updated existing stop/pump tests to the new closure/pump signatures.

## Validations (all pass)
- `cargo clippy --all-targets -- -D warnings`
- `cargo fmt --check`
- `cargo test` (175 + 2 + 6 passed)
- `cargo run -- check` ⇒ OK
