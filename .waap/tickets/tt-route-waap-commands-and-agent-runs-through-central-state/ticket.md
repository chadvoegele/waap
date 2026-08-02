+++
name = "Route waap commands and agent runs through central state"
creation_date = 2026-08-02T23:45:59Z
status = "pending"
depends_on = ["tt-add-serialized-central-waap-state-transactions", "tt-implement-legacy-waap-state-migration-repair", "tt-implement-central-waap-check-and-remote-freshness-validation"]
+++

Activate central state across all waap commands as specified in `specs/waap-state-worktree.md`.

## Scope

- Refactor command dispatch so ticket and agent reads/writes use `ProjectContext.state_root`; application source operations use the invocation/source worktree.
- Route all mutations through serialized central-state transactions and return commit hashes from branch `waap` without moving application HEADs.
- Run central pre-validation for state-aware commands and preserve setup-only `init` plus explicit `repair` behavior.
- Ensure commands from the primary checkout and multiple linked worktrees observe the same state immediately.
- Make `agent run` create its source worktree from the invoking application HEAD, never from `waap`; reject invocation from the state worktree when no application source HEAD was selected.
- Keep runner prompts and backend repository roots tied to source context while reading agent instructions from the resolved state path. Tell agents to mutate state through the CLI, not direct file edits.
- Remove nested `.waap` project resolution and other superseded legacy behavior.
- Add integration tests covering every command from primary/linked/state worktrees, state commit placement, source branch selection, override semantics, and concurrent transitions.

## Self-hosting safety

Do not install the newly built binary globally or run its state-changing commands against this repository during an agent run. The repository remains on legacy state until the final operator-controlled cutover ticket.
