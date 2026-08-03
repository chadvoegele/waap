# waap

`waap` is an agent automation platform that structures disposable-agent work as tickets and agents. It stores state as plain Markdown with TOML frontmatter and validates it with `waap check`.

## Central state

Each non-bare Git repository has one central state worktree on a local orphan branch named `waap`. The state directory is derived from the canonical primary checkout path:

```text
<primary repository root>
  /home/chad/code/github.com/chadvoegele/waap
<state directory>
  ~/.local/state/waap/data/home/chad/code/github.com/chadvoegele/waap
```

The state directory is the root of the state worktree and contains only:

```text
agents/
tickets/
```

All primary and linked **source worktrees** of one repository resolve the same state directory. State reads, validation, writes, staging, and commits occur there on `waap`; application source operations occur in the invoking source worktree. State commits must not move application branch heads.

`.waap` refers only to legacy state in an application checkout. It is not a current state location and must not be copied into source worktrees.

### State resolution

`--waap-root` always names a state directory containing `agents` and `tickets`, never an application checkout.

- Except for `waap init`, an explicit `--waap-root <path>` is canonicalized and used directly. Waap does not derive, relocate, or compare another state location.
- `waap init --waap-root <path>` treats `<path>` as the exact new state-directory target.
- Without `--waap-root`, waap derives the state path under `~/.local/state/waap/data/` from the current source worktree's primary checkout. `HOME` must be absolute.

An invocation from the central state worktree cannot select a source HEAD for `waap agent run`; invoke it from an application source worktree instead.

### Setup, migration, and relocation

`waap init` is setup-only. For a new repository it creates the central worktree, `agents` and `tickets`, and a parentless `waap init` commit. When `origin/waap` already exists, it verifies and adopts that state instead. It leaves the application checkout unchanged and reports `state_directory` in human and JSON output.

`waap init` fails without modification if selected central state or legacy `.waap` exists. `waap repair` is the explicit recovery command:

- It migrates legacy `.waap` into central state, commits the central migration, then removes and commits the legacy state on its application branch.
- It repairs a moved primary checkout's registered state worktree and relocates it to the newly derived directory.
- It repairs `origin/waap` upstream configuration.

When central state and legacy `.waap` coexist, waap does not choose or merge them. Reconcile them manually, then run `waap repair`. Run relocation repair from the primary checkout.

### Remote behavior

With `origin`, local `waap` tracks `origin/waap`, but waap never pushes automatically. Synchronization is manual, for example `git -C "$WAAP_STATE" push`.

`waap check` fetches `origin/waap`. Remote-only commits, including divergence, produce a warning without failing an otherwise valid check. The warning does not fetch application branches, merge, rebase, or reconcile state.

## Records

Agents and tickets are state records. Their paths below are relative to the central state directory.

### Agents

Agent instructions and metadata are stored at `agents/<agent-id>/agent.md`; optional progress notes may be stored at `agents/<agent-id>/work_log.md`. Agent IDs are `aa-` plus a name slug or eight lowercase hex characters. Agent frontmatter is strict:

```toml
+++
name = "List Tickets Developer"
creation_date = 2026-06-18T15:00:34Z
status = "ready" # ready, running, completed, failed, aborted
session_id = "ses_9032dd..." # added after start
system = "opencode" # opencode, claude, codex; added after start
+++

# Purpose
Implement code for `tt-list-tickets`
```

### Tickets

Tickets are stored at `tickets/<ticket-id>/ticket.md`. IDs are `tt-` plus a name slug or eight lowercase hex characters. Slugs are lowercase ASCII, trim whitespace, replace spaces with one hyphen, remove punctuation, and are shorter than 64 characters. Long slugs are truncated and given a random four-hex suffix; conflicts use the same suffix strategy.

```toml
+++
name = "List Tickets"
creation_date = 2026-06-18T10:15:02Z
status = "pending" # pending, in-progress, completed, abandoned
depends_on = ["tt-required-foundation"]
+++

Implement ticket listing.
```

`depends_on` is optional. A ticket is blocked until all dependencies are completed. The legacy `title` field is accepted when reading old tickets, but new writes use `name`.

## CLI

The CLI is the required interface for normal state mutations. Mutations validate, write, stage explicit state paths, and commit on `waap` under a per-repository lock.

```sh
waap init
waap check
waap repair
```

`waap init` and `waap check` report the absolute state directory in human-readable output and as `state_directory` in JSON. `waap check` validates central-worktree and state-record invariants and requires a clean state worktree. It does not stage, commit, repair, or reconcile state.

Emergency direct edits are permitted only as recovery:

1. Edit the central state worktree, never legacy `.waap` or a source worktree.
2. Run `waap check`; a dirty-worktree failure is expected until the edit is committed. Correct all content errors.
3. Explicitly commit the corrected state files on branch `waap`.
4. Run `waap check` again and require a clean success.

### Tickets

```sh
waap ticket new --name "Implement Example Feature" --depends-on tt-foundation < ticket.md
waap ticket get --ticket-id tt-example-feature
waap ticket update --ticket-id tt-example-feature --set-status in-progress
waap ticket update --ticket-id tt-example-feature --add-depends-on tt-other
waap ticket update --ticket-id tt-example-feature --remove-depends-on tt-other
waap ticket list --status pending --unblocked
```

`ticket new` accepts repeatable `--depends-on`. `ticket update` requires `--ticket-id` and at least one of `--set-status`, `--add-depends-on`, or `--remove-depends-on`. `ticket list` accepts `--status`, `--blocked`, or `--unblocked`; the latter two conflict.

### Agents

```sh
waap agent new --name "Example Developer" < agent.md
waap agent get --agent-id aa-example-developer
waap agent update --agent-id aa-example-developer --set-status completed
waap agent list --status ready
waap agent run --agent-id aa-example-developer --system codex
waap agent stop --agent-id aa-example-developer
```

`agent run` supports `opencode` (default), `claude`, and `codex`. `agent stop` without `--agent-id` stops all running agents.

`--output-format json` and `--waap-root` are global options and may appear after the subcommand. JSON reports use `ticket_id` and `agent_id` fields for scripting.

### Agent worktrees

`waap agent run` commits its state transition on `waap`, then creates an isolated **source** worktree at `worktrees/<agent-id>` relative to the application repository's primary checkout. It starts that worktree from the invoking application's HEAD, not `waap`, and removes it after the run.

The launcher owns source-worktree creation, cleanup, and state transitions. Prompts may point an agent to its central `agent.md`, but agents must use CLI commands for state changes and must not edit central state directly or create/remove source worktrees themselves. Detailed agent instructions own source integration policy.

### Agent systems

OpenCode runs against its authenticated HTTP API. It uses `OPENCODE_SERVER_MODEL` as `provider/model` or `provider/model/variant`; recognized variants are `none`, `minimal`, `low`, `medium`, `high`, `xhigh`, and `max`.

Claude runs `claude -p` with a minted session ID, JSON output, bypassed permissions, and optional `$CLAUDE_MODEL`. Codex is selected with `waap agent run --system codex`.

## Bundled skill and bootstrap

The waap skill and role templates are:

```text
.agents/skills/waap/SKILL.md
.agents/skills/waap/roles/planner/agent.md
.agents/skills/waap/roles/developer/agent.md
```

From an application source checkout, create and run a planner:

```sh
planner_id=$(waap --output-format json agent new < .agents/skills/waap/roles/planner/agent.md | jq -r '.agent_id')
waap agent run --agent-id "$planner_id"
```

Resolve `${ticket_id}` in the developer template, then create and run it:

```sh
developer_id=$(waap --output-format json agent new < resolved-developer-agent.md | jq -r '.agent_id')
waap agent run --agent-id "$developer_id"
```

Multiple developers may run in parallel when their tickets are unblocked.
