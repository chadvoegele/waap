+++
status = "draft"
created = 2026-11-26
last_update = 2026-11-27
+++

# Agent prompt delivery

## Purpose

Use one WAAP command to send a prompt to an agent. The caller can start a turn, steer the current turn, or let WAAP choose based on whether a turn is active.

## Context

Today, `agent new` stores a prompt in `agent.md`. `agent run` sends a reference to that file, creates a temporary execution worktree, and only accepts a `ready` agent. This proposal supersedes those parts of the [main spec](spec.md) and [backend spec](agent-system-backend.md), and replaces the standalone CLI in the [steering spec](agent-steering.md). Existing live steering delivery and acknowledgement semantics still apply.

## Goals

- Register an agent and select its backend without opening a session.
- Send the first prompt and later prompts through `agent prompt`, preserving the backend session across turns.
- Let callers choose delivery explicitly and leave execution worktree management to the agent or caller.

## Non-goals

- Queued follow-ups (`--delivery=queue`) and image input (`--image`); both are future scope.
- Creating empty sessions at registration, switching backends mid-session, or emulating resume by replaying history.
- Changing WAAP state storage, recovering stale `running` records, or retrying `failed` agents in the initial release.

## Specification

```sh
waap agent new --system pi --name 'Keyboard navigation'
# Agent ID: aa-keyboard-navigation
waap agent prompt --agent-id aa-keyboard-navigation --delivery=start < task.md
# From another terminal, while a turn is active:
waap agent prompt --agent-id aa-keyboard-navigation --delivery=steer --text='Focus on keyboard navigation.'
# Omit --delivery to start or steer based on whether a turn is active:
waap agent prompt --agent-id aa-keyboard-navigation --text='Check the accessibility notes.'
```

```text
Registration ───→ Agent record
Prompt input ───→ Delivery routing ───→ Turn owner ───→ Backend session
                       │                    ▲
                       └──→ Live control ───┘
```

### Registration and input

`agent new [--system <backend>] [--name <name>]` must generate and return a WAAP agent ID and write a metadata-only `agent.md` with `creation_date`, `status = "ready"`, and `system`. The default is Pi. Registration must not read stdin, start a backend, or assign a backend `session_id`.

Naming is unchanged. With `--name`, WAAP stores the supplied name in `name` and generates an `aa-` ID from its slug using the existing length and collision rules. Without `--name`, WAAP generates an `aa-` ID with eight random hexadecimal characters and omits `name`. Later commands address the agent by ID.

`agent prompt --agent-id <id> [--delivery=<mode>] [--text=<text>]` must use `--text` if supplied, otherwise read redirected stdin to EOF. It must reject missing or whitespace-only input without changing state or contacting the backend, and must not wait for interactive input. It must preserve the text and send it as prompt content, without writing it to `agent.md` or constructing a file-reference prompt.

### Delivery modes

Omitting `--delivery` must behave exactly like `--delivery=asap`.

| Mode | Agent can start a turn | Turn active |
| --- | --- | --- |
| `start` | Start a new turn. | Reject without steering. |
| `steer` | Reject without starting a turn. | Deliver to the current turn using native steering. |
| `asap` | Act as `start`. | Act as `steer`. |

Route each request once. If the chosen turn ends before live delivery, the live owner is unreachable, or the backend does not support steering, return an error rather than falling back to another mode. If delivery may already have occurred, report uncertainty without automatically retrying. A stale `running` record, or an owner not yet accepting steering requests, must not be treated as idle. `failed` agents are outside the initial delivery scope.

### Starting and resuming turns

For a `ready` agent, `start` must create a session on the registered backend, persist the adapter's session ID, and send the first prompt. For a `completed` or `aborted` agent, `start` must reopen the session identified by the stored `system` and `session_id` and send the prompt as a new turn. Missing metadata, a missing session, or an unsupported resume operation must result in an error, not a replacement session. WAAP must not require a backend to accept the WAAP agent ID as its session ID.

### Agent lifecycle

`running` means an invocation owns the turn, including startup and settlement. It does not guarantee that the backend is ready to accept steering. `completed`, `aborted`, and `failed` record the latest outcome, not whether the backend session still exists.

| From | Action or event | To |
| --- | --- | --- |
| No record | `agent new` registers the agent. | `ready` |
| `ready` | `prompt` takes the `start` path and claims the first turn. | `running` |
| `completed` or `aborted` | `prompt` takes the `start` path and claims a follow-up with a saved session ID. | `running` |
| `ready` | `agent stop --agent-id <id>` cancels the agent before its first turn. | `aborted` |
| `running` | `prompt` takes the `steer` path and delivers to the active turn. | `running` (unchanged) |
| `running` | The turn finishes successfully. | `completed` |
| `running` | The owner confirms the turn was stopped after `agent stop` or an interrupt. | `aborted` |
| `running` | First-session startup or turn execution fails. | `failed` |
| `running` | A follow-up fails to reopen its saved session before submission. | Prior `completed` or `aborted` status |

WAAP must claim `running` atomically before starting backend work. Only one invocation may own a turn. Its owner stays attached until settlement and persists the actual outcome. Accepting a stop request alone does not end the turn; if natural completion wins the race, record `completed`. Stopping a `ready` agent leaves no session to resume. `agent stop` without an ID targets only `running` agents.

This table lists all lifecycle transitions. Rejected commands and metadata updates do not change status; `failed` has no outgoing transition in this release.

### Execution directory

Use the resolved repository root as the backend's working directory. WAAP must not create, reuse, or remove a Git execution worktree on an agent's behalf. Agents or callers that need isolation must manage it themselves. A resumed session restores conversation history, not a filesystem snapshot.

## Compatibility

Backward compatibility with the previous agent record format and CLI structure is not required. No state migration, legacy record reader, or command aliases are required. `agent prompt` replaces `agent run` and `agent steer`; callers that require steering only must use `--delivery=steer`. Scripts must move prompt redirection from `new` to `prompt` and select the backend at `new`. Leave historical execution worktrees and branches untouched.

Backend resume may ship incrementally. Pi is the proposed first backend; each backend must demonstrate exact-session resume before advertising support. Existing per-turn model and reasoning options should remain available when starting turns without changing the registered backend.

## Future scope

`--delivery=queue` will accept follow-up prompts only while a turn is active, for delivery as a new turn after the current one finishes. Queue lifecycle and durability are outside this specification.

## Verification

- Verify registration with and without `--name`, including stored names, slug collisions, and generated IDs. Verify that `new` records the explicit or default backend without reading stdin or opening a session.
- Exercise every lifecycle transition, including stop before first start, natural completion racing a stop, and rollback after a failed reopen. Verify that steering, rejected commands, and metadata updates leave status unchanged.
- Verify exact text delivery from `--text` and redirected Markdown. Reject missing or blank input and `--image` without changing state. Verify no prompt content is written to `agent.md`.
- Start a first turn, complete it, and start a follow-up. Verify that it recalls information from the first turn and preserves the session ID. Repeat after an explicit stop with a saved session ID.
- Verify all three delivery modes against active and idle agents, including that omitting `--delivery` behaves like explicit `asap` in both cases. Verify that `asap` does not fall back after failed steering and that a turn finishing during live delivery cannot silently trigger a new turn. Verify that `--delivery=queue` is rejected without changing state.
- Verify that concurrent requests cannot create overlapping turns, and that unavailable sessions and unsupported operations never create a replacement session.
- Verify the working directory and that no path creates or removes an execution worktree.
