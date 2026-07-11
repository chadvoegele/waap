# waap

`waap` Agent Automation Platform

## Description

`waap` is an agent automation platform that structures execution of disposable AI agents. Tickets are used to describe work, and agents are used to perform work. It provides scaffolding and parallelization for sub-agents.

`waap` can be used to build an "AI software factory".

## Agent Purposes

You may give `waap` agents different roles to solve your problem. An agent's purpose is carried by its instructions/content, not by a separate metadata field.

For a software development project, consider
- Planner
	- Translates application specifications, e.g. `specs.md`, into an implementation plan via tickets.
- Developer
	- Implements their assigned ticket.
    - Merges their changes into the application repository.
    - Tests the application meets the specification for the ticket scope.

## Conventions

All paths in the specification are relative to the application's repository root.

## Datastore

waap maintains its own datastore within the application's repository at `/.waap/`.

## Agents

Agents do the work in waap. Agents use the repository source code and specifications primarily as context, and use `.waap/` to track their own state and log partial progress.

Agent state is tracked in the `/.waap/agents/` directory. An agent can have an optional human-readable name. Its id is `aa-` plus the name slug, or an 8-character random hex value when unnamed.

Agent metadata is stored in TOML frontmatter.

**Agent Schema**
```
+++
name = "List Tickets Developer"
creation_date = 2026-06-18T15:00:34Z
status = "ready"  # ready, running, completed, failed, aborted
session_id = "ses_9032dd..."  # add after agent is started
system = "opencode"  # opencode, claude; add after agent is started
+++

# Purpose
Implement code for `tt-list-tickets`
```

Example:
`/.waap/agents/aa-list-tickets-developer/agent.md`

Agents should also keep a work log, recording any work done. 

Example:
`/.waap/agents/aa-list-tickets-developer/work_log.md`

## Tickets

Tickets track agent work, capture partial progress, and anchor the developer lifecycle. They serve as a bridge between the specifications and the implementation, resolving ambiguity in the specifications while not fully code.

Tickets are stored in the `/.waap/tickets/` directory, typically as Markdown files.

Ticket metadata is stored in TOML frontmatter.

**Ticket Schema**
```
+++
name = "List Tickets"
creation_date = 2026-06-18T10:15:02Z
status = "pending"  # pending, in-progress, completed, abandoned
+++

# Spec Reference
Line 10-15 of /specs/spec.md

# Description
List tickets in `.waap/tickets/` directory. Return in JSON format.
Additionally allow a flag to filter by status.
```

Tickets can have an optional human-readable name. A named ticket id is the name slug prefixed by `tt-`; an unnamed ticket id is `tt-` plus an 8-character random hex value. The legacy `title` metadata field is accepted as `name` when reading old tickets, but new writes use `name`.

Slug Requirements:
1. Lowercase
1. Leading and trailing spaces are trimmed
1. Spaces translated to hyphens
1. Repeated hyphens are collapsed to a single hyphen
1. Punctuation removed
1. Avoid special and non-ASCII characters
1. Less than 64 characters
    1. excess is truncated, and appended with a dash and random 4 character hex hash
    1. Example: `tt-list-all-very-long-tickets-that-are-too-long-for-a-slug-de32`

If a named ticket or agent id conflicts, append a 4-character random hex suffix.

Example:
`/.waap/tickets/tt-list-tickets/ticket.md`

Agents should update the `ticket.md` file with any key decisions, important commands, code fragments so that another agent can resume the work if necessary.

## Command Line Interface (CLI)

The `waap` CLI is the primary interface for interacting with waap state, though the state can be edited manually so long as it is still valid schema, e.g. passes `waap check`.

- Global Parameters
    - `--output-format`  # Choices: json, human-readable
    - `--waap-root`  # Existing project directory inside a git repository

The default format is human-readable.

Mutation commands must not leave `/.waap/` invalid when they return success. Manual edits can
still invalidate state; `waap check` remains the validator for finding and repairing such changes.

Without `--waap-root`, commands use the nearest ancestor containing `.waap/`, bounded by the
current git root. If none exists, commands use the git root so `waap init` can initialize it and
other commands can report missing state.

### waap check
Validates `/.waap` state.

Missing `/.waap` state is invalid and reports that `waap init` is required.

1. `/.waap` has the correct directory structure.
1. For `/.waap/agents/`,
    1. Every sub directory is an agent id.
    1. Each agent directory has a `agent.md` file.
    1. If there is a work log, it is named `work_log.md`.
    1. Other files are allowed in the agent directory.
1. For `/.waap/tickets/`,
    1. Every sub directory is a ticket id.
    1. Each ticket directory has a `ticket.md` file.
1. Agent frontmatter is valid.
1. Ticket frontmatter is valid.

Frontmatter is validated strictly: unknown fields outside the documented agent and ticket schemas are rejected. The error names the offending field and the record path so it can be located. Optional known fields such as `depends_on` remain optional.

### waap ticket
- new
    - Creates a new ticket
    - Prepends TOML frontmatter in ticket.md
    - Appends ticket.md contents from stdin
    - Commits the ticket.md to git
    - Parameters
        - Optional
            - `--name`
            - `--depends-on`  # Existing ticket id; repeatable
    - Streams
        - stdin: write to ticket markdown
        - stdout: reports created ticket path, metadata, and file size
- update
    - Updates an existing ticket
    - The ticket name and id cannot be changed.
    - Commits the ticket.md changes to git
    - Parameters
        - Required
            - `--ticket-id`
        - At least one of these
            - `--set-status`
            - `--add-depends-on`  # Existing ticket id; repeatable
            - `--remove-depends-on`  # Valid ticket id; repeatable; absent dependencies are ignored
    - Streams
        - stdout: reports updated ticket path, metadata, and file size
- get
    - Gets an existing ticket
    - Parameters
        - Required
            - `--ticket-id`
    - Streams
        - stdout: reports ticket metadata and content
- list
    - Lists existing tickets with filter criteria applied.
    - Parameters
        - Optional
            - `--status`  # returned list has this status
    - Streams
        - stdout: reports ticket ids

### waap agent
- new
    - Creates a new agent entry in `/.waap/agents/`
    - Prepends TOML frontmatter in agent.md
    - Appends agent.md contents from stdin
    - Commits the agent.md to git
    - Parameters
        - Optional
            - `--name`
    - Streams
        - stdin: write to agent markdown
        - stdout: reports created agent path, metadata, and file size
- run
    - Starts the agent harness
    - Updates agent entry to running
    - Commits the agent.md changes to git
    - Parameters
        - Required
            - `--agent-id`
        - Optional
            - `--system`  # agent system used to run the agent: opencode, claude. Defaults to opencode.
    - Streams
        - stdout: reports agent path, metadata
- stop
    - Stops running agents
    - Marks their statuses as 'aborted'
    - Commits the agent.md changes to git
    - Parameters
        - Optional
            - `--agent-id`  # if not provided, all agents are stopped
- update
    - Updates an existing agent
    - The agent name and id cannot be changed.
    - Commits the agent.md changes to git
    - Parameters
        - Required
            - `--agent-id`
        - At least one of these
            - `--set-status`
            - `--set-session-id`
    - Streams
        - stdout: reports updated agent path, metadata
- get
    - Gets an existing agent
    - Parameters
        - Required
            - `--agent-id`
    - Streams
        - stdout: reports agent metadata and content
- list
    - Lists existing agents, with filter criteria applied.
    - Parameters
        - Optional
            - `--status`  # returned list has this status
    - Streams
        - stdout: reports agent ids


## Running Agents

Agents can be run with different agent systems, selected with `waap agent run --system`. The chosen system and the resulting session id are recorded in the agent metadata.

### Worktree Lifecycle

`waap agent run` owns the agent worktree lifecycle. It first commits the agent's `running` status and chosen system to `main`, then creates an isolated git worktree at `worktrees/$agent_id` (a fresh branch named after the agent) and launches the system inside it. Agents rebase their branch onto the current `main` `HEAD` and merge with `--ff-only` before finishing. Finally, the worktree is removed.

### opencode (default)

opencode runs against a remote server through its authenticated HTTP API. `waap` creates the session against the canonical repository root with denied interactive permissions, establishes the repository-wide SSE event stream, then submits the worktree-directed goal to `POST /session/{sessionID}/prompt_async`. The prompt uses `$OPENCODE_SERVER_MODEL` as `provider/model`, the `build` agent, and a text part. Matching completed text, tool, step, and error events are forwarded as JSON lines; matching idle status completes the run. OpenCode is attached to the durable repository project, while the goal requires all implementation, Git, and validation work to occur in the isolated agent worktree.

### claude

claude runs as a local headless process (`claude -p`). `waap` mints the session id itself and passes it via `--session-id`. The goal is passed directly as the prompt. The model is optional and read from `$CLAUDE_MODEL`.

```
claude -p \
  --session-id "$session_id" \
  --output-format json \
  --permission-mode bypassPermissions \
  --model "$CLAUDE_MODEL" \
  "Complete when instructions in /.waap/agents/${agent_id}/agent.md are satisfied"
```

## waap Skill

In addition to the CLI, waap also includes an agent skill that tells a (non-waap spawned) agent how to interact with waap. The skill includes a CLI reference as well as instructions to be passed to the waap agents.

```
- .agents/skills/waap/SKILL.md
- .agents/skills/waap/roles/planner/agent.md
- .agents/skills/waap/roles/developer/agent.md
```

### Software Development Bootstrap

To start the waap software lifecyle, first run a planner agent to create tickets as needed.

```
cd ${APP_REPO_ROOT}
planner_id=$(cat .agents/skills/waap/roles/planner/agent.md | waap --output-format json agent new | jq -r '."agent-id"')
waap agent run --agent-id ${planner_id}
```

Then select a ticket for an agent, insert it into the developer agent.md template, and create an agent with those instructions,
```
cd ${APP_REPO_ROOT}
# replace ${ticket_id} in .agents/skills/waap/roles/developer/agent.md to create resolved_developer_agent.md
developer_id=$(cat resolved_developer_agent.md | waap --output-format json agent new | jq -r '."agent-id"')
waap agent run --agent-id ${developer_id}
```

Multiple developer agents can be run in parallel if there are enough tickets.
