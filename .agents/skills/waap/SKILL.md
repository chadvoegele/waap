---
name: waap
description: waap is a structured agent automation platform for tickets, directed acyclic workflows, disposable agents, and durable Git-backed state. Use for waap workflows, agents, tickets, validation, or the waap CLI.
---

# waap

## Central state and safety

Waap keeps one central state worktree per application repository. For a primary checkout at `/home/me/code/example`, state is at:

```text
~/.local/state/waap/data/home/me/code/example
```

That directory, on local orphan branch `waap`, contains `agents/` and `tickets/` directly. It is not an application source worktree. Every linked source worktree of the same repository resolves it, and all CLI state commits remain on `waap`.

`.waap` is legacy state in an application checkout only. Do not create or edit it for a central-state project.

Use the CLI for normal mutations. Do not directly edit central state or create/remove source worktrees. `waap agent run` owns source-worktree creation and cleanup and starts the agent source worktree from the invoking application's HEAD, never `waap`.

If emergency recovery requires a direct state edit:

1. Edit the central state directory.
2. Run `waap check`; it must fail while the worktree is dirty. Fix every content error it reports.
3. Explicitly commit the corrected files on branch `waap` in the state worktree.
4. Run `waap check` again and require a clean success.

`waap check` never commits, stages, merges, or repairs direct edits.

## Setup and recovery

From an application source checkout, initialize a new project:

```sh
waap init
waap check
```

`waap init` is setup-only. It creates or adopts state and reports its absolute state directory; it refuses an existing central directory or legacy `.waap`. `waap init` and `waap check` include `state_directory` in JSON output. Use `waap repair` to migrate legacy state, repair a moved state worktree, or restore the `origin/waap` upstream.

`--waap-root` always names a state directory containing `agents` and `tickets`, never an application checkout:

```sh
waap --waap-root /srv/waap-state ticket list
waap init --waap-root /srv/new-waap-state
```

The first command uses the supplied directory directly. The second creates state at the supplied exact target. Without it, waap derives `~/.local/state/waap/data/...` from the current source worktree's primary checkout.

If central state and legacy state coexist, reconcile them manually before `waap repair`; waap will not choose or merge them. After moving an application repository, run `waap repair` from its primary checkout.

With `origin`, local `waap` tracks `origin/waap`, but synchronization and pushing are manual. `waap check` fetches to warn about remote-only or diverged state; the warning does not reconcile either branch.

## Basic workflow

Create a ticket from Markdown:

```sh
cat cool-feature.md | waap ticket new --name 'Cool Feature'
```

Create an agent from instructions, then run it:

```sh
cat agent.md | waap agent new --name 'Cool Feature Developer'
waap agent run --agent-id aa-cool-feature-developer
```

Use the generated IDs from JSON when scripting:

```sh
ticket_id=$(waap --output-format json ticket new --name 'Cool Feature' < cool-feature.md | jq -r '.ticket_id')
agent_id=$(waap --output-format json agent new --name 'Cool Feature Developer' < agent.md | jq -r '.agent_id')
waap agent run --agent-id "$agent_id"
```

Ticket IDs begin `tt-`; agent IDs begin `aa-`. Their records are at `$WAAP_STATE/tickets/<ticket-id>/ticket.md` and `$WAAP_STATE/agents/<agent-id>/agent.md`, where `$WAAP_STATE` is the state directory reported by `waap check`.

## Ticket dependencies

A ticket is blocked until every ticket in `depends_on` is completed. Specify repeatable dependencies at creation or update them later:

```sh
waap ticket new --name 'Deploy' --depends-on tt-build --depends-on tt-test < deploy.md
waap ticket update --ticket-id tt-deploy --add-depends-on tt-security-review
waap ticket update --ticket-id tt-deploy --remove-depends-on tt-test
waap ticket list --status pending --unblocked
```

## Agent instructions

Agent instructions should identify the ticket and require CLI state mutation. They must not tell the agent to edit central state files or create source worktrees. A suitable instruction is:

> Implement the functionality described by ticket `tt-example`. Use the waap CLI for state changes. Work only in the source worktree prepared by the launcher; do not create or remove worktrees.

Read records with CLI commands when needed:

```sh
waap ticket get --ticket-id tt-example
waap agent get --agent-id aa-example
```

Templates are available at:

- [Planner](./roles/planner/agent.md)
- [Developer](./roles/developer/agent.md)

## CLI reference

```sh
waap check
waap ticket new --name 'Example' < ticket.md
waap ticket get --ticket-id tt-example
waap ticket update --ticket-id tt-example --set-status in-progress
waap ticket list --status pending --unblocked
waap agent new --name 'Example Agent' < agent.md
waap agent get --agent-id aa-example
waap agent update --agent-id aa-example --set-status completed
waap agent list --status ready
waap agent run --agent-id aa-example --system codex
waap agent stop --agent-id aa-example
```

`agent run` supports `opencode` (default), `claude`, and `codex`. For OpenCode, set `OPENCODE_SERVER_URL`, `OPENCODE_SERVER_USERNAME`, `OPENCODE_SERVER_PASSWORD`, and `OPENCODE_SERVER_MODEL`. The model accepts `provider/model` or `provider/model/variant`; recognized variants are `none`, `minimal`, `low`, `medium`, `high`, `xhigh`, and `max`.

## Running outside a sandbox

State-mutating CLI commands write and commit state. Run them outside a command sandbox that blocks Git locks.

- **Codex:** use `sandbox_permissions: "require_escalated"` with a concise justification.
- **Claude Code:** use `dangerouslyDisableSandbox: true` on the Bash call.

## waap loop

1. Create tickets for work.
2. While pending tickets are unblocked:
   1. Create agents with clear instructions.
   2. Run agents.
   3. Run application tests and `waap check`.
   4. Add tickets for remaining work.

For a software factory, first create a planner ticket for the application specification, run its planner agent, then use the same loop for the resulting developer tickets.
