+++
status = "draft"
created = 2026-10-20
last_update = 2026-10-20
+++

# Agent steering

## Purpose

Let users redirect a running waap agent without stopping or restarting it.
The first implementation supports Codex; other backends explicitly reject steering.

## Context

Codex runs through an app-server connection owned by the live `waap agent run`
process. A separate CLI invocation cannot write to that connection directly.
Steering therefore needs a local control interface to the existing run process,
which retains ownership of the Codex thread, active turn, and protocol stream.

![Steering data flows, interfaces, and data model](agent-steering.svg)

## Goals

- Submit a prompt to a running Codex agent's current turn.
- Report acceptance or failure to the caller.
- Preserve the existing session, worktree, and lifecycle.
- Reject unsupported backends explicitly.

## Non-goals

- Starting, resuming, restarting, or stopping an agent.
- Editing agent instructions or ticket content.
- Steering Claude or OpenCode in this PR; support may follow in future PRs.
- Broadcasting, durable message queues, or automatic delivery retries.
- Repairing stale lifecycle metadata or guaranteeing that an agent follows a prompt.

## Specification

### CLI

```sh
waap agent steer --agent-id "$agentid" --prompt "$prompt"
```

Both flags are required. The prompt must contain at least one non-whitespace
character. Validation must not trim or otherwise alter the submitted text.
Multiline text, Unicode, quotes, and shell metacharacters must be preserved.
The prompt is data, not a shell command.

The command must resolve the agent in the current repository's waap state and
reject unknown IDs or records whose status is not `running` before delivery.
Persisted `running` status is sufficient for the status check; no separate
process-liveness probe is required. Stale records fail through delivery rather
than triggering lifecycle repair.

Dispatch must use the record's persisted backend, not a caller-selected backend.
Existing legacy backend-default behavior must remain unchanged. Claude and
OpenCode must return a nonzero exit code with:

```text
steering not implemented for backend <backend>
```

### Delivery and acknowledgement

For Codex, the steering command must send the unchanged prompt to the live run
process through a repository- and run-scoped local control interface. The run
process must submit it on its existing app-server connection using `turn/steer`
with `threadId`, `expectedTurnId`, and a text `input` item. The expected turn ID
must be the active turn owned by that run, not a new or resumed turn.

Success means Codex acknowledged acceptance of the steering request. Merely
connecting or locally queueing a request is not success. The command must not
wait for the agent to act on the message or finish its turn. Human-readable
output must identify the agent and confirm acceptance; JSON output must identify
`agent_id` and indicate `accepted: true`, following existing output conventions.

The live run must continue processing Codex events while accepting control
requests. Responses must be correlated to their requests so simultaneous steering
calls cannot receive each other's acknowledgements or interfere with run output.

Missing control endpoints, startup before an active turn exists, exited runs,
completed turns, transport errors, Codex rejection, and acknowledgement timeouts
must produce a nonzero exit code and a clear diagnostic. Connection and
acknowledgement waits must be bounded. If delivery may have occurred but cannot
be confirmed, the diagnostic must say acceptance is unknown. There must be no
automatic retry, fallback to `turn/start`, or delivery to a later run.

### Interfaces and data model

The following are logical interface contracts, not mandated Rust type names:

| Interface | Input | Result |
| --- | --- | --- |
| CLI / shared steer orchestration | Agent ID, prompt | Validate record and select persisted backend |
| Backend steering operation | Repository scope, agent ID, session ID, prompt | Accepted or descriptive error; unsupported by default |
| Local run control request | Request ID, expected session/run identity, prompt | Correlated acceptance or error |
| Codex app-server request | `turn/steer { threadId, expectedTurnId, input: [{ type: "text", text: prompt }] }` | Protocol acknowledgement or error |

Existing persistent agent metadata remains unchanged: `status`, `system`, and
`session_id` retain their meanings; Codex `session_id` is its thread ID. Steering
must not rewrite these fields, modify `agent.md`, or create a waap state commit.

Transient run-control state associates the repository and agent ID with a unique
run identity and local endpoint. The owner holds the session/thread ID, active
turn ID, and in-flight request correlations. The endpoint must not be advertised
as ready before it can handle requests, and must be removed on normal exit.
Leftover endpoints after a crash must not cause delivery to a different run.
A new run must have a distinct identity even when reusing the same agent ID.

Local control access must be restricted to the current OS user. Endpoint lookup
must isolate repositories with identical agent IDs. Received requests must be
validated against the owning run identity and session; stale or mismatched
requests must be rejected. Prompts must not be added to routine diagnostics.

### Compatibility

Existing run, stop, state, and worktree behavior must remain unchanged. Steering
must not mark a run completed, aborted, or otherwise alter its lifecycle status,
including on delivery failure. No persistent state migration is required.

## Verification

- CLI tests: required flags, whitespace-only rejection, exact preservation of
  nonempty text, and human/JSON success output.
- Orchestration tests: unknown agent, every non-running status, persisted backend
  dispatch, unsupported Claude/OpenCode, and no state changes or commits.
- Protocol tests: correct `turn/steer` payload, expected active turn ID, Codex
  acceptance/rejection, and response correlation amid unrelated notifications.
- Local-control integration tests: deliver to an already-running fake Codex
  app server through the run owner, without starting another session or turn;
  concurrent requests; bounded timeouts; exit during submission; startup and
  completed-turn rejection; and ambiguous acknowledgement loss without retry.
- Isolation/lifecycle tests: same agent ID in two repositories, stale endpoint,
  old-run identity after restart, restricted endpoint access, and normal cleanup.
- Regression tests: run output, stop interruption, completion, and worktree cleanup
  continue to work while steering requests are processed.

## Open questions

- Choose the local IPC transport and endpoint layout during implementation
  (for example, a user-restricted Unix-domain socket).
- Select and document concrete connection and acknowledgement timeout values.
- Verify `turn/steer` against the supported Codex app-server version; servers
  without that method must fail clearly rather than emulate steering by restarting.
