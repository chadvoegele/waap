---
name: waap
description: waap is a structured agent automation platform for specifying workflows as tickets in directed acyclic graphs (DAGs), running agents to execute the workflow in parallel, and durably tracking the instructions, agent instructions and progress. Use when coordinating agents, creating or running waap workflows, or validating or managing waap agents or tickets via the `waap` CLI.
---

# waap

## Basic Workflow

First, specify to-be-completed work in waap tickets. The work is described in markdown format, e.g. `cool_feature.md`.

```sh
$ cat cool_feature.md | waap ticket new --name "Cool Feature"
```

This creates a ticket at `.waap/tickets/<ticket-id>/ticket.md`. Ticket IDs start with `tt-` and use lowercase filesystem-safe slugs, e.g. `tt-cool-feature`.

Next, create a waap agent to complete the work in the ticket. The agent's instructions are also described in markdown format, e.g. `agents.md`, and should indicate to work on the ticket.

```sh
$ cat agents.md | waap agent new
Agent ID: aa-1234abcd
```
This creates an agent at `.waap/agents/<agent-id>/agent.md`. Agent IDs start with `aa-` followed by eight lowercase hex characters.

Then run the agent with `waap agent run --agent-id aa-1234abcd`.

## State Model

waap stores state in the application repository under `.waap/`, and tracks it with `git`.

All state is stored as plain text, but should be validated with `waap check` to ensure `waap` commands will work.

### Running Outside the Sandbox

WAAP commands that mutate state (`ticket new/update`, `agent new/run/update`) must run outside the agent sandbox. WAAP writes files before committing them, so a sandbox-blocked Git lock can leave untracked state behind. Disable the sandbox on the first attempt rather than letting the command fail and retrying.

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

> Your role is to implement the functionality described in `.waap/tickets/${ticket_id}/ticket.md`.

When appropriate the instructions should also include to merge their changes back to the repository.

If your agents are repeating the same function, consider storing the common instructions as a reusable template. Some example templates are at:

- [Planner](./roles/planner/agent.md)
    - Creates waap tickets for implementing a program defined in spec.md
- [Developer](./roles/developer/agent.md)
    - Implement functionality described in a waap ticket.

It is helpful to ensure continuity across agents, to also instruct the agent to update:

1. An agent specific work log: `.waap/agents/<agent-id>/work_log.md`
    1. A chronological log summarizing the work the agent did during their session.
1. Top-level shared `AGENTS.md`
    1. Include any project-specific details that might be helpful for futures agents like:
        1. key files/artifact locations
        2. developer commands, e.g. building, formatting, linting, testing, etc.
        3. common gotchas
        4. project conventions

## Agent Parallelization

`waap agent run` owns the agent worktree lifecycle. Before launching the selected system, it creates an isolated git worktree (`worktrees/<agent-id>`) and runs the agent inside it; after the system process exits it removes that worktree. This isolates each agent's changes so many agents can run in parallel, without relying on the agent to create or clean up its own worktree.

## Agent Frontmatter

Agent metadata is stored as TOML frontmatter at the top of the agent markdown file.

```toml
+++
creation_date = 2026-06-18T15:00:34Z
status = "ready"
session_id = "ses_9032dd..."
system = "opencode"
+++
```

The `system` property identifies which system was used to run the agent, and the `session_id` is the system's unique identifier for the agent run.

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

Create an agent from an instruction prompt on stdin:

```sh
waap agent new < .agents/skills/waap/roles/planner/agent.md
waap agent new < resolved-developer-agent.md
```

Run an agent:

```sh
waap agent run --agent-id aa-1234abcd
```

Run an agent with a different system:

```sh
waap agent run --agent-id aa-1234abcd --system claude
waap agent run --agent-id aa-1234abcd --system codex
waap agent run --agent-id aa-1234abcd --system opencode
```

For OpenCode, set `OPENCODE_SERVER_URL`, `OPENCODE_SERVER_USERNAME`, `OPENCODE_SERVER_PASSWORD`, and `OPENCODE_SERVER_MODEL`. The model accepts `provider/model` or `provider/model/variant`; recognized variants are `none`, `minimal`, `low`, `medium`, `high`, `xhigh`, and `max`.

Update agent status:

```sh
waap agent update --agent-id aa-1234abcd --set-status completed
```

Prefer `--output-format json` when scripting and parse the `agent_id` or `ticket_id` fields from command output.

## waap Loop

To effectively use waap, follow this loop:

1. Create tickets for work to be done. (`cat ticket.md | waap ticket new ...`)
1. While there are unblocked tickets, (`waap ticket list --status pending --unblocked`)
    1. Create agents to complete the work. (`cat agent.md | waap agent new ...`)
    2. Run the agents. (`waap agent run ...`)
    3. Add new tickets as needed. (`cat new_ticket.md | waap ticket new ...`)

### Software Factory Loop

To build a software factory, follow this loop:

1. Draft a `spec.md` including any diagrams, or associated files describing the program.
1. Create a planner ticket to create tickets for implementing the program. (`cat ticket.md | waap ticket new ...`)
1. Run the planner agent. (`waap agent run ...`)
1. While there are unblocked tickets, (`waap ticket list --status pending --unblocked`)
    1. Create agents to complete the work. (`cat agent.md | waap agent new ...`)
    1. Run the agents. (`waap agent run ...`)
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
