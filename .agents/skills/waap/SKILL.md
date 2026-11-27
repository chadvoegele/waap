---
name: waap
description: waap is a structured agent automation platform for specifying workflows as tickets in directed acyclic graphs (DAGs), running agents to execute the workflow in parallel, and durably tracking the instructions, agent instructions and progress. Use when coordinating agents, creating or running waap workflows, or validating or managing waap agents or tickets via the `waap` CLI.
---

# waap

## Central State

Run `waap init` from the application checkout. The state directory (`${waap_data}` below) defaults to `~/.local/state/waap/data/` plus the primary checkout’s absolute path without its leading slash. Use `--waap-root <path>` to override it. Run `waap check` to report the path and validate the records.

If you move the application repository, use `--waap-root` to keep the existing state path. To move a plain state directory instead, stop agents and other waap commands, use `waap check` from the new checkout to find the destination, then move the directory and recheck.

## Basic Workflow

First, specify to-be-completed work in waap tickets. The work is described in markdown format, e.g. `cool_feature.md`.

```sh
$ cat cool_feature.md | waap ticket new --name "Cool Feature"
```

This creates a ticket at `${waap_data}/tickets/<ticket-id>/ticket.md`. Ticket IDs start with `tt-` and use lowercase filesystem-safe slugs, e.g. `tt-cool-feature`.

Next, register an agent and send it instructions that refer to the ticket. Put those instructions in a separate Markdown file, for example `task.md`.

```sh
agent_id=$(waap --output-format json agent new --system pi | jq -er '.agent_id')
waap agent prompt --agent-id "$agent_id" --delivery=start < task.md
```

Registration writes metadata at `${waap_data}/agents/<agent-id>/agent.md`, without reading stdin or opening a session. IDs start with `aa-` and use the `--name` slug if provided, or eight lowercase hex characters otherwise. Always use the returned ID.

From another terminal, steer the active turn with `waap agent prompt --agent-id "$agent_id" --delivery=steer --text='Focus on keyboard navigation.'`. Steering supports Pi and Codex; Claude and OpenCode reject it. If the owner disappears or acceptance is unknown, inspect the agent before sending anything else; WAAP does not retry or reroute.

Stop the agent from another terminal, or omit `--agent-id` to stop all running agents in this repository:

```sh
waap agent stop --agent-id "$agent_id"
waap agent stop
```

## State Model

Use the waap CLI to change plain-text records. To track state in Git, use a checkout you manage and commit changes yourself.

### Running Outside the Sandbox

Run waap commands outside the sandbox if it blocks writes to the state directory, including the lock.

- **Codex:** set `sandbox_permissions: "require_escalated"` on the command call and provide a concise `justification`.
- **Claude Code:** set `dangerouslyDisableSandbox: true` on the `Bash` tool call.

## Ticket Dependencies

If a ticket (B) depends on another ticket (A), then ticket (B) is blocked until ticket (A) is completed. This dependency is specified when the ticket is created with `waap ticket new --depends-on tt-A`, where `tt-A` is ticket id of the blocking ticket (A). Multiple blocking tickets can be specified with `--depends-on` multiple times. This can be used to create arbitrary DAGs of work.

To see all unblocked pending tickets, use:

```sh
waap ticket list --status pending --unblocked
```

## Ticket Frontmatter

Ticket metadata is stored as TOML frontmatter at the top of the ticket markdown file.

```toml
+++
name = "Implement Example Feature"
creation_date = 2026-06-18T10:15:02Z
status = "pending"
depends_on = ["tt-required-foundation"]
+++
```

`depends_on` is optional. When specified, it's an array.

## Agent Instructions

The agent instructions should include the work to be completed, typically by referring to a waap ticket id.

> Your role is to implement the functionality described in `${waap_data}/tickets/${ticket_id}/ticket.md`.

When appropriate the instructions should also include to merge their changes back to the repository.

If your agents are repeating the same function, consider storing the common instructions as a reusable template. Some example templates are at:

- [Planner](./roles/planner/agent.md)
    - Creates waap tickets for implementing a program defined in spec.md
- [Developer](./roles/developer/agent.md)
    - Implement functionality described in a waap ticket.

It is helpful to ensure continuity across agents, to also instruct the agent to update:

1. An agent specific work log: `${waap_data}/agents/<agent-id>/work_log.md`
    1. A chronological log summarizing the work the agent did during their session.
1. Top-level shared `AGENTS.md`
    1. Include any project-specific details that might be helpful for futures agents like:
        1. key files/artifact locations
        2. developer commands, e.g. building, formatting, linting, testing, etc.
        3. common gotchas
        4. project conventions

## Agent Parallelization

`waap agent prompt --delivery=start` starts a turn at the root of the launch checkout, even from a subdirectory of a linked worktree. The [Developer role](./roles/developer/agent.md) creates and removes its own worktree.

## Agent Frontmatter

Agent metadata is stored as TOML frontmatter at the top of the agent markdown file.

```toml
+++
creation_date = 2026-06-18T15:00:34Z
status = "ready"
system = "pi"
+++
```

`system` is the backend selected at registration. WAAP adds `session_id` when it opens the backend session and keeps that ID across resumed turns. Prompts and backend turn IDs are not written to the record.

## CLI Reference

Validate waap state:

```sh
waap check
```

Create a ticket from Markdown on stdin:

```sh
waap ticket new --name "Implement Example Feature" --depends-on tt-foundation < ticket.md
```

List unblocked pending tickets:

```sh
waap ticket list --status pending --unblocked
```

Update ticket status:

```sh
waap ticket update --ticket-id tt-example-feature --set-status in-progress
waap ticket update --ticket-id tt-example-feature --set-status completed
```

Register an agent, then send a role prompt. Choose `pi` (the default), `codex`, `claude`, or `opencode` at registration; the backend cannot be changed later.

```sh
planner_id=$(waap --output-format json agent new --name Planner | jq -er '.agent_id')
waap agent prompt --agent-id "$planner_id" --delivery=start < .agents/skills/waap/roles/planner/agent.md

developer_id=$(waap --output-format json agent new --system codex | jq -er '.agent_id')
waap agent prompt --agent-id "$developer_id" --delivery=start \
  --model gpt-5.4 --reasoning-effort high < resolved-developer-agent.md
```

`--model` and `--reasoning-effort` apply to starts only. If omitted, `--delivery` defaults to `asap`: start if possible, steer if `running`. Use explicit `start` or `steer` to avoid the other action. Use `--text` or redirected stdin; interactive and blank input are rejected. Queue and image input are not supported.

A later start on a `completed` or `aborted` agent requires a saved session. Exact resume is supported by Pi 1.5.2 from the original launch checkout with the same session store. WAAP will not fork the session or replace missing history. A ready agent stopped before its first turn has no session; register another agent for a fresh start.

- Codex:
  - `CODEX_MODEL`: overridden by `--model`.
  - `CODEX_REASONING_EFFORT`: overridden by `--reasoning-effort`.
- Pi:
  - `WAAP_PI_MODEL`: overridden by `--model`.
  - `WAAP_PI_REASONING_EFFORT`: overridden by `--reasoning-effort`.
  - `WAAP_PI_BIN`: selects the executable.

Accepted efforts are `minimal`, `low`, `medium`, `high`, `xhigh`, and `max`.
Codex also accepts `none` and `ultra`; Pi also accepts `off`.

For OpenCode, set `OPENCODE_SERVER_URL`, `OPENCODE_SERVER_USERNAME`, `OPENCODE_SERVER_PASSWORD`, and `OPENCODE_SERVER_MODEL`. The model accepts `provider/model` or `provider/model/variant`; recognized variants are `none`, `minimal`, `low`, `medium`, `high`, `xhigh`, and `max`.

Rename an agent without changing its ID or status:

```sh
waap agent update --agent-id aa-1234abcd --set-name 'New display name'
```

WAAP records the turn's outcome automatically; do not update agent lifecycle status yourself.

Prefer `--output-format json` when scripting and parse the `agent_id` or `ticket_id` fields from command output.

## waap Loop

To effectively use waap, follow this loop:

1. Create tickets for work to be done. (`cat ticket.md | waap ticket new ...`)
1. While there are unblocked tickets, (`waap ticket list --status pending --unblocked`)
    1. Register agents to complete the work. (`waap agent new ...`)
    2. Send each agent its task. (`waap agent prompt --agent-id ... --delivery=start < task.md`)
    3. Add new tickets as needed. (`cat new_ticket.md | waap ticket new ...`)

### Software Factory Loop

To build a software factory, follow this loop:

1. Draft a `spec.md` including any diagrams, or associated files describing the program.
1. Create a planner ticket to create tickets for implementing the program. (`cat ticket.md | waap ticket new ...`)
1. Register a planner and send it the planning instructions with `waap agent prompt --delivery=start`.
1. While there are unblocked tickets, (`waap ticket list --status pending --unblocked`)
    1. Register developers. (`waap agent new ...`)
    1. Send each developer its instructions. (`waap agent prompt --agent-id ... --delivery=start < task.md`)
    1. Run end-to-end tests.
    1. Verify all aspects of the spec have been implemented.
    1. Add new tickets as needed to address spec drift, testing, or refactoring. (`cat new_ticket.md | waap ticket new ...`)

### Structured Agent Program Loop

To run a structured program leveraging agents, e.g.

```
1. Fetch configs from config store
1. For every host in fleet,
    1. Deploy or update configs for that host
    1. Restart the host
    1. Ensure the host is back online
```

First, create tickets for implementing the program, making sure to include dependency details, then run the waap loop.
