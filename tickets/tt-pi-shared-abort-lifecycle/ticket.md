+++
name = "Pi shared abort lifecycle"
creation_date = 2026-08-07T17:19:54Z
status = "completed"
+++

# Implement shared aborted lifecycle semantics

Implement the shared lifecycle changes required by the Pi backend specification before adding Pi itself.

Specification: `specs/pi-agent-system.md` on PR branch `docs/pi-agent-system-spec` (PR #6). Read the complete spec before editing.

## Scope

- Add an explicit aborted/interrupted backend run outcome.
- Map Codex interrupted turns to that outcome instead of failed.
- Make run and stop orchestration converge idempotently on `aborted` in either process order.
- Ensure the original run exits nonzero after interruption without an `aborted -> failed` or `failed -> aborted` transition error.
- Preserve worktree cleanup, reports, commits, and error propagation.
- Reject `agent stop` for a `running` record with no persisted `system`; leave it running, do not resolve a backend, and report invalid metadata.
- Keep ready-agent stop behavior unchanged.
- Factor shared owner-process SIGTERM/pkill behavior only if it improves the implementation without unrelated refactoring.

## Acceptance criteria

- Focused unit tests cover Codex interruption, both run/stop observation orders, idempotent aborted persistence, owner exit code, and missing-system rejection.
- Existing Opencode, Claude, Codex, run, and stop behavior remains covered.
- Required repository validation passes.
- Changes are committed, integrated into `docs/pi-agent-system-spec`, pushed to PR #6, and this ticket is completed.
