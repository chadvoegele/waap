+++
status = "draft"
created = 2026-11-26
last_update = 2026-11-26
+++

# Agent prompt delivery

## Purpose

Use one WAAP command to send a prompt to an agent. The caller chooses whether to start a turn, steer the current turn, or queue a follow-up.

## Context

Today, `agent new` stores a prompt in `agent.md`. `agent run` sends a reference to that file, creates a temporary execution worktree, and only accepts a `ready` agent. This proposal supersedes those parts of the [main spec](spec.md) and [backend spec](agent-system-backend.md), and replaces the standalone CLI in the [steering spec](agent-steering.md). Existing live steering delivery and acknowledgement semantics still apply.

## Goals

- Register an agent and select its backend without opening a session.
- Send the first prompt and later prompts through `agent prompt`, preserving the backend session across turns.
- Let callers choose delivery explicitly and leave execution worktree management to the agent or caller.

## Non-goals

- Image input (`--image`) or other non-text inputs.
- Creating empty sessions at registration, switching backends mid-session, or emulating resume by replaying history.
- Changing WAAP state storage, recovering stale `running` records, or retrying `failed` agents in the initial release.

## Specification

```sh
waap agent new --system pi
# Agent ID: aa-1234abcd
waap agent prompt --agent-id aa-1234abcd --delivery=start < task.md
# From another terminal, while a turn is active:
waap agent prompt --agent-id aa-1234abcd --delivery=steer --text='Focus on keyboard navigation.'
waap agent prompt --agent-id aa-1234abcd --delivery=queue --text='Add a regression test.'
# To start or steer based on whether a turn is active:
waap agent prompt --agent-id aa-1234abcd --delivery=asap --text='Check the accessibility notes.'
```

```text
Registration ───→ Agent record
Prompt input ───→ Delivery routing ───→ Turn owner ───→ Backend session
                       │                    ▲
                       ├──→ Live control ───┤
                       └──→ Follow-up queue ┘
```

### Registration and input

`agent new [--system <backend>]` must generate and return a WAAP agent ID and write a metadata-only `agent.md` with `creation_date`, `status = "ready"`, and `system`. The default is Pi. Registration must not read stdin, start a backend, or assign a backend `session_id`. The new interface omits `--name`; the generated agent ID is the handle for later commands.

`agent prompt --agent-id <id> --delivery=<mode> [--text=<text>]` must use `--text` if supplied, otherwise read redirected stdin to EOF. It must reject missing or whitespace-only input without changing state or contacting the backend, and must not wait for interactive input. It must preserve the text and send it as prompt content, without writing it to `agent.md` or constructing a file-reference prompt.

### Delivery modes

| Mode | Agent can start a turn | Turn active |
| --- | --- | --- |
| `start` | Start a new turn. | Reject without steering or queueing. |
| `steer` | Reject without starting a turn. | Deliver to the current turn using native steering. |
| `queue` | Reject; there is no active turn to queue behind. | Queue a new turn for after the active turn settles. |
| `asap` | Act as `start`. | Act as `steer`. |

Route each request once. If the chosen turn ends before live delivery, the live owner is unreachable, or the backend does not support steering, return an error rather than falling back to another mode. If delivery may already have occurred, report uncertainty without automatically retrying. A stale `running` record, or an owner not yet accepting steering or queue requests, must not be treated as idle. `failed` agents are outside the initial delivery scope.

### Starting and resuming turns

For a `ready` agent, `start` must create a session on the registered backend, persist the adapter's session ID, and send the first prompt. For a `completed` or `aborted` agent, `start` must reopen the session identified by the stored `system` and `session_id` and send the prompt as a new turn. Missing metadata, a missing session, or an unsupported resume operation must result in an error, not a replacement session. WAAP must not require a backend to accept the WAAP agent ID as its session ID.

Only one owner may start turns for an agent at a time. WAAP must claim `running` atomically before starting backend work. The owner remains attached and records the final outcome as `completed`, `aborted`, or `failed`. If reopening fails before submission, restore the prior status and keep the session identity. `agent update` must not be able to claim a turn.

### Queued follow-ups

An accepted queue request is acknowledged as **queued**, not delivered. After normal turn completion, the owner must start queued prompts as new turns in acceptance order on the same backend session, without injecting text into the current turn. The turn owner must serialize settlement, queue acceptance, and starting the next turn so that it cannot acknowledge a request after it has stopped processing follow-ups. An explicit stop must not start another turn from the queue.

### Execution directory

Use the resolved repository root as the backend's working directory. WAAP must not create, reuse, or remove a Git execution worktree on an agent's behalf. Agents or callers that need isolation must manage it themselves. A resumed session restores conversation history, not a filesystem snapshot.

## Compatibility

`agent prompt` replaces `agent run` and `agent steer`; steering callers must select `--delivery=steer`. Scripts must move prompt redirection from `new` to `prompt` and backend selection to `new`. Existing records stay readable: retain legacy `name` and Markdown bodies, but never use them as prompt input. A legacy `ready` record without a backend or session ID uses the default backend on first start. Leave historical execution worktrees and branches untouched.

Backend resume may ship incrementally. Pi is the proposed first backend; each backend must demonstrate exact-session resume before advertising support. Existing per-turn model and reasoning options should remain available when starting turns without changing the registered backend.

## Open questions

- Should `--delivery` remain required as shown, or should `asap` be the default?
- Should the queue exist only for the lifetime of the live owner, or survive process exit? How should callers learn that accepted but undelivered entries were discarded after a failure or stop?

## Verification

- Verify that `new` records the explicit or default backend without reading stdin or opening a session.
- Verify exact text delivery from `--text` and redirected Markdown. Reject missing or blank input and `--image` without changing state. Verify no prompt content is written to `agent.md`.
- Start a first turn, complete it, and start a follow-up. Verify that it recalls information from the first turn and preserves the session ID. Repeat after an explicit stop with a saved session ID.
- Verify all four delivery modes against active and idle agents. Verify that `asap` does not fall back after failed steering and that a turn finishing during live delivery cannot silently trigger a new turn.
- Queue multiple prompts and verify acknowledgement before delivery, FIFO new turns on the same session, and no queued turn starting after stop.
- Verify that concurrent requests cannot create overlapping turns, and that unavailable sessions and unsupported operations never create a replacement session.
- Verify the working directory and that no path creates or removes an execution worktree.
