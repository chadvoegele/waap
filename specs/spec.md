# waap

waap Agent Automation Platform

## Description

waap leverages agents to transform specifications into a tested, production-ready application. waap works by using agents to automate the software development lifecycle:

1. Product specifications
1. Backlog curation
1. Implementation
1. Review
1. Testing
1. Release

waap is an attempt at building an "AI software factory".

## Conventions

All paths in the specification are relative to the application's repository root.

## Specifications

The application specifications are defined in the `/specs/` directory, typically as Markdown files and optionally diagrams, schemas, or other related resources.

Example:
```
- /specs/spec.md
- /specs/spec.png
```

## Datastore

waap maintains its own datastore within the application's repository at `/.waap/`.

## Agent Roles

waap uses different agent roles to perform each stage of the software development lifecycle.

- Planner
	- Translates application specifications into an implementation plan via tickets.
- Developer
	- Implements their assigned ticket.
    - Merges their changes into the application repository.
    - Tests the application meets the specification for the ticket scope.

## Agents

Agents do the work in waap. Agents use the repository source code and specifications primarily as context, and use `.waap/` to track their own state and log partial progress.

Agent state is tracked in the `/.waap/agents/` directory. An agent is identified by a 8 character random hex hash, prefixed by `aa-`.

Agent metadata is stored in TOML frontmatter.

**Agent Schema**
```
+++
creation_date = 2026-06-18T15:00:34Z
role = "developer"  # developer, planner
status = "ready"  # ready, running, completed, aborted
session_id = "ses_9032dd..."  # add after agent is started
system = "opencode"  # opencode, claude; add after agent is started
+++

# Purpose
Implement code for `tt-list-tickets`
```

Example:
`/.waap/agents/aa-3881fda0/agent.md`

Agents should also keep a work log, recording any work done. 

Example:
`/.waap/agents/aa-3881fda0/work_log.md`

## Tickets

Tickets track agent work, capture partial progress, and anchor the developer lifecycle. They serve as a bridge between the specifications and the implementation, resolving ambiguity in the specifications while not fully code.

Tickets are stored in the `/.waap/tickets/` directory, typically as Markdown files.

Ticket metadata is stored in TOML frontmatter.

**Ticket Schema**
```
+++
title = "List Tickets"
creation_date = 2026-06-18T10:15:02Z
status = "pending"  # pending, in-progress, completed, abandoned
+++

# Spec Reference
Line 10-15 of /specs/spec.md

# Description
List tickets in `.waap/tickets/` directory. Return in JSON format.
Additionally allow a flag to filter by status.
```

The ticket id should be a filesystem-safe version of ticket title, i.e. a slug, prefixed by literal `tt-`.

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

If there is a conflicting ticket id, the new ticket should similarly append a 4 character hash.

Example:
`/.waap/tickets/tt-list-tickets/ticket.md`

Agents should update the `ticket.md` file with any key decisions, important commands, code fragments so that another agent can resume the work if necessary.

## Command Line Interface (CLI)

The `waap` CLI is the primary interface for interacting with waap state, though the state can be edited manually so long as it is still valid schema, e.g. passes `waap check`.

- Global Parameters
    - `--output-format`  # Choices: json, human-readable
    - `--repo-root`  # Path to the application repository root that contains `.waap/`; defaults to current directory

The default format is human-readable.

### waap check
Validates `/.waap` state.

Missing state is considered valid. Only validate directories and files that already exist.

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

### waap ticket
- new
    - Creates a new ticket
    - Prepends TOML frontmatter in ticket.md
    - Appends ticket.md contents from stdin
    - Parameters
        - Required
            - `--title`
    - Streams
        - stdin: write to ticket markdown
        - stdout: reports created ticket path, metadata, and file size
- update
    - Updates an existing ticket
    - The ticket title cannot be changed since it's also the id.
    - Parameters
        - Required
            - `--ticket-id`
        - At least one of these
            - `--set-status`
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
    - Parameters
        - Required
            - `--role`
    - Streams
        - stdin: write to agent markdown
        - stdout: reports created agent path, metadata, and file size
- run
    - Starts the agent harness
    - Updates agent entry to running
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
    - Parameters
        - Optional
            - `--agent-id`  # if not provided, all agents are stopped
- update
    - Updates an existing agent
    - The agent id cannot be changed.
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

Agents can be run with different agent systems, selected with `waap agent run --system`. Each system is given the same goal: complete the instructions in `/.waap/agents/$agent_id/agent.md`. The chosen system and the resulting session id are recorded in the agent metadata.

### opencode (default)

opencode runs against a remote server. Create a session, then submit a goal command attached to that session.

```
opencode run --attach "$OPENCODE_SERVER_URL" \
  --username "$OPENCODE_SERVER_USERNAME" \
  --password "$OPENCODE_SERVER_PASSWORD" \
  --model "$OPENCODE_SERVER_MODEL" \
  --dir "$REPO_ROOT" \
  --agent build \
  --command goal \
  --format json \
  "Complete when instructions in /.waap/agents/${agent_id}/agent.md are satisfied"
```

Extract the opencode session id from the response and record it in the agent metadata.

### claude

claude runs as a local headless process (`claude -p`) in the repository root. There is no remote server: waap mints the session id itself (a UUID) and passes it via `--session-id`, so the same id can be recorded in the agent metadata. The goal is passed directly as the prompt. `--permission-mode auto` lets the agent act without interactive prompts. The model is optional and read from `$CLAUDE_MODEL`; claude handles its own authentication.

```
claude -p \
  --session-id "$session_id" \
  --output-format json \
  --permission-mode auto \
  --model "$CLAUDE_MODEL" \
  "Complete when instructions in /.waap/agents/${agent_id}/agent.md are satisfied"
```

claude has no remote session to abort, but the session id is part of the local process's command line (`--session-id <uuid>`), so `waap agent stop` aborts a claude agent by signalling the matching local process (e.g. `pkill -TERM -f <session_id>`) before marking it `aborted`.


## waap Skill

In addition to the CLI, waap also includes an agent skill that tells a (non-waap spawned) agent how to interact with waap. The skill includes a CLI reference as well as instructions to be passed to the waap agents.

```
- .agents/skills/waap/SKILL.md
- .agents/skills/waap/roles/planner/agent.md
- .agents/skills/waap/roles/developer/agent.md
```

### .agents/skills/waap/roles/planner/agent.md

Your role is to develop a plan to implement the application according to its specifications, captured in `/specs`. The plan should be captured as tickets in `/.waap/tickets/` directory.

Begin by reviewing the application's source code and tests, as well as previously completed tickets. Then review the application's specifications. Where some application functionality is described in the specifications but absent in the application's implementation, create tickets to implement the missing functionality. The tickets should divide the work into manageable sections that fit within the developer agent's context window (~128K tokens), will be merge-able without unresolvable conflicts, and should resolve some ambiguity in the specification as needed to make a cohesive application.

**Ticket Schema**
...    # replace with ticket schema described above

Verify the ticket.md syntax with `waap check`.

If the application is fully implemented, consider whether it is also fully tested with appropriate unit test, and end-to-end tests. Ticket can also be used to ensure appropriate test coverage.

If the application is fully implemented and tested, complete your `/goal`.

After completing your `/goal`, mark your status as 'completed' with `waap agent update --agent-id $agent_id --set-status completed`.

### .agents/skills/waap/roles/developer/agent.md

Your role is to implement code for the functionality described in `/.waap/tickets/${ticket_id}/ticket.md`. After implementing the code, write adequate unit tests, and end-to-end tests if appropriate. Once the code is tested, merge it, resolving conflicts as necessary.

Be aware that many agents are editing the code simultaneously. Use git worktree's to isolate your changes before merging.

**git worktree example**
```
worktree_dir=worktrees/${agent_id}
git worktree add -b ${agent_id}/${ticket_id} ${worktree_dir}
pushd ${worktree_dir}
# make changes, test, and merge
popd
git worktree remove ${worktree_dir}
```

Before you start, mark your ticket as in-progress with `waap ticket update --ticket-id $ticket_id --set-status in-progress`.

Include your agent id and the ticket id your worked on in your commit message. Use a fast-forward merge when possible to keep a linear history.

When your code is merged and tested, complete your `/goal`.

If the ticket is already completed or abandoned, complete your `/goal`.

After completing your `/goal`,
1. mark your ticket as 'completed' with `waap ticket update --ticket-id $ticket_id --set-status completed`.
1. mark your status as 'completed' with `waap agent update --agent-id $agent_id --set-status completed`.

### Bootstrap

To start the waap software lifecyle, first run a planner agent to create tickets as needed.

```
cd ${APP_REPO_ROOT}
planner_id=$(cat .agents/skills/waap/roles/planner/agent.md | waap --output-format json agent new --role planner | jq -r '."agent-id"')
waap agent run --agent-id ${planner_id}
```

Then select a ticket for an agent, insert it into the developer agent.md template, and create an agent with those instructions,
```
cd ${APP_REPO_ROOT}
# replace ${ticket_id} in .agents/skills/waap/roles/developer/agent.md to create resolved_developer_agent.md
developer_id=$(cat resolved_developer_agent.md | waap --output-format json agent new --role developer | jq -r '."agent-id"')
waap agent run --agent-id ${developer_id}
```

Multiple developer agents can be run in parallel if there are enough tickets.
