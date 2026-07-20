+++
name = "Require agent stop to abort running agents"
creation_date = 2026-07-19T12:14:06Z
status = "in-progress"
+++

## Problem

`waap agent update --set-status aborted` currently permits a running agent to transition to `aborted` without terminating its backend session. This can leave an OpenCode, Claude, or Codex session running while WAAP metadata incorrectly reports that the agent is aborted.

## Requirements

- Reject `waap agent update --agent-id <id> --set-status aborted` when the agent is currently `running`.
- Direct users to `waap agent stop --agent-id <id>` in the error message.
- Preserve the core `running -> aborted` lifecycle transition so `waap agent stop` can terminate the backend session and then record the agent as aborted.
- Preserve existing valid update behavior, including `ready -> aborted` if supported by the lifecycle.
- Ensure a rejected update does not modify the agent record or create a WAAP state commit.

## Acceptance Criteria

- An update attempting `running -> aborted` fails with an actionable error referencing `waap agent stop`.
- The running agent metadata and Git history remain unchanged after rejection.
- `waap agent stop` still transitions a running agent to `aborted` after invoking backend termination behavior.
- Tests cover the rejected update and the preserved stop behavior.
- All repository validations pass.
