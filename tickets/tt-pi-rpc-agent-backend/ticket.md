+++
name = "Pi RPC agent backend"
creation_date = 2026-08-07T17:19:54Z
status = "pending"
depends_on = ["tt-pi-shared-abort-lifecycle"]
+++

# Implement the Pi RPC agent backend

Implement the direct Pi Coding Agent backend specified by `specs/pi-agent-system.md` on PR branch `docs/pi-agent-system-spec` (PR #6). Read the complete spec and the shared lifecycle implementation before editing.

## Scope

- Add `AgentSystem::Pi`, persisted label `pi`, and backend construction.
- Make Pi the default for `waap agent run` when `--system` is omitted.
- Accept the existing `--model` and `--reasoning-effort` options for Pi and Codex only. Translate Pi `none` to `off`, map `minimal` through `max`, and reject Pi `ultra` before state changes.
- Implement `WAAP_PI_BIN`, `WAAP_PI_MODEL`, and `WAAP_PI_REASONING_EFFORT` precedence and validation.
- Spawn one local `pi --mode rpc` child in the agent worktree with persistent sessions, `--approve`, and a WAAP session name.
- Remove inherited parent-session metadata variables while preserving Pi config/auth and the rest of the environment.
- Implement strict LF-only JSONL framing, response correlation, interleaved event handling, bounded command startup timeouts, and child cleanup/reaping.
- Obtain `get_state.data.sessionId`, return it from backend start, and defer prompt submission until shared orchestration persists it.
- Wait for `agent_settled`, map final assistant stop reasons exactly as specified, forward assistant text deltas only, and cancel dialog-style extension UI requests.
- Translate owner SIGTERM into exactly one Pi RPC abort and return the shared aborted outcome.
- Keep record, commit, report, ticket, and worktree lifecycle logic outside the Pi adapter.

## Acceptance criteria

- Fake-process protocol tests cover command construction, config precedence, environment scrubbing, framing edge cases, interleaved events/responses, session-before-prompt ordering, UI cancellation, completion mapping, abort, malformed/failed/timeout/EOF paths, and process reaping.
- CLI/frontmatter/backend tests cover Pi selection, Pi as default, explicit other systems, and option ownership.
- No real provider is called by the default test suite.
- Required repository validation passes.
- Changes are committed, integrated into `docs/pi-agent-system-spec`, pushed to PR #6, and this ticket is completed.
