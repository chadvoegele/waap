+++
status = "draft"
created = 2026-11-26
last_update = 2026-11-26
+++

# Starting and steering agent turns

## Purpose

Let users send an initial prompt and later follow-ups to the same WAAP agent without rewriting its record. Keep starting a new turn distinct from steering one already in progress.

## Context

Today, `agent new` stores a prompt in `agent.md`. `agent run` sends a reference to that file, creates a temporary execution worktree, and only accepts a `ready` agent. This proposal supersedes those parts of the [main spec](spec.md) and [backend spec](agent-system-backend.md). The [steering spec](agent-steering.md) still governs live delivery and acknowledgement.

## Goals

- Register an agent and select its backend without starting a backend session.
- Use one command for the first turn and later turns in the same session.
- Keep steering explicit and leave execution worktree management to the agent or caller.

## Non-goals

- Creating empty backend sessions at registration, switching backends mid-session, or replaying conversation history to emulate resume.
- Changing WAAP state storage, ticket workflows, or the steering control protocol.
- Recovering stale `running` records or retrying `failed` agents in the initial release.

## Specification

```sh
waap agent new --system pi
# Agent ID: aa-1234abcd
waap agent start --agent-id aa-1234abcd < task.md
# From another terminal, while a turn is active:
waap agent steer --agent-id aa-1234abcd --prompt 'Focus on keyboard navigation.'
# After the turn ends:
waap agent start --agent-id aa-1234abcd --prompt 'Add a regression test.'
```

```text
new   → agent record
start → prompt input → turn owner → backend session → new turn
steer → live control socket ─────→ turn owner      → active turn
```

### Register an agent

`agent new [--system <backend>]` must generate and return a WAAP agent ID and write a metadata-only `agent.md` with `creation_date`, `status = "ready"`, and `system`. The default backend remains Pi. Registration must not read stdin, start a backend, or assign a backend `session_id`. `--name` is not part of the new interface; the generated agent ID is the handle for later commands.

### Start a turn

`agent start --agent-id <id> [--prompt <text>]` must use the flag if supplied; otherwise it must read redirected stdin to EOF. It must reject missing or whitespace-only input without changing the record or contacting the backend, and must not wait for interactive input. It must preserve the supplied text and pass it directly as a user message, without writing it to the agent record or constructing a file-reference prompt.

| Agent status | Behavior |
| --- | --- |
| `ready` | Create the first session on the registered backend and submit the prompt as the first turn. |
| `completed` or `aborted` | Reopen the session identified by the stored `system` and `session_id`, then submit the prompt as a new turn. If either is missing or the session cannot be reopened, fail. |
| `running` | Fail and direct the caller to `agent steer`. |
| `failed` | Fail; retry behavior is outside the initial scope. |

WAAP must use the session ID obtained by the backend adapter and persist it on the first start. It must not require a backend to accept the WAAP agent ID as a session ID. A follow-up must preserve the existing session ID. A backend without resume support must report that limitation and must not create a replacement session or replay the initial prompt.

Only one `start` invocation may own an active turn for an agent. WAAP must claim the `running` status atomically before starting backend work. The owner remains attached until settlement and records the outcome as `completed`, `aborted`, or `failed`. Rejecting a command before claiming the turn must not change its status. If reopening a session fails before prompt submission, WAAP must retain the prior status and session identity. If submission may already have occurred, WAAP must not automatically retry or send a second copy. The new transitions back to `running` are specific to `start`, not `agent update`.

### Steer an active turn

`agent steer --agent-id <id> --prompt <text>` must use the existing live control path. It must fail if there is no active turn, if the turn ends before delivery, or if the backend does not support steering. It must never create or reopen a session. `agent stop` continues to stop the active run through the backend's existing stop behavior.

### Execution directory

Use the resolved repository root as the backend's working directory for both initial and later turns. WAAP must not create, reuse, or remove a Git execution worktree on an agent's behalf. Agents or callers that need isolation must manage it themselves. A resumed session restores conversation history, not a filesystem snapshot; parallel agents may share the checkout.

## Compatibility

`agent start` replaces `agent run`. Scripts must move their prompt redirection from `new` to `start` and move backend selection to `new`. Existing records stay readable: retain legacy `name` and Markdown bodies, but never use them as new-turn input. A legacy `ready` record without a backend or session ID uses the default backend on first start. Historical execution worktrees and branches are left untouched.

Backend resume may ship incrementally. Pi is the proposed first backend; each backend must demonstrate exact-session resume before advertising support. Existing per-run model and reasoning options should move to `start` without changing the registered backend.

## Verification

- Register an agent with an explicit backend and with the default. Verify that `new` does not consume stdin or contact a backend, and writes only metadata with no session ID.
- Start with `--prompt` and with redirected Markdown. Verify exact text delivery, rejection of missing or blank input, and no prompt content written to `agent.md`.
- Complete a turn, then start another with the same agent ID. Verify that the backend session ID is unchanged and the follow-up can recall information from the first turn. Verify the same resume path after an explicit stop with a saved session ID.
- Exercise missing or unavailable sessions and a backend without resume support. Verify a nonzero exit and no replacement session. Verify that simultaneous starts cannot both submit turns for one agent.
- Steer a live turn and verify that `start` rejects it. Verify that steering a finished turn fails without submitting a new one.
- Verify the working directory on first start and resume, and that neither path creates or removes a Git worktree.
