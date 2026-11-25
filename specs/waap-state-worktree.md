+++
status = "final"
created = 2026-11-25
last_update = 2026-11-25
+++

# Shared waap state directory

## Purpose

Share agent and ticket records across application worktrees without requiring Git history for state.

## Goals

- Resolve one default state directory per application repository.
- Leave state versioning to the user.
- Preserve atomic record writes and coordination between waap commands.

## Non-goals

- Migration of legacy state or repair of Git worktrees.
- Synchronization across separate clones or hosts.
- Per-record optimistic concurrency or changes to agent execution worktrees.

## Specification

Commands run from an application Git checkout. Without an override, they resolve the same state directory from the primary checkout and its linked worktrees: `~/.local/state/waap/data/` followed by the canonical primary checkout path without its leading slash. `--waap-root <path>` overrides that directory and may point outside Git or at a user-managed checkout. Relative overrides are relative to the invocation directory.

```text
CLI -> resolve state path -> acquire StateLock -> read and validate records
                                                          |
                                                          v
                                     report result <- atomically write changes
```

`waap init` creates missing `agents/` and `tickets/` directories and reports the resolved path. Existing records stay intact. `waap check` reports the state path and validates the records on disk regardless of Git status.

State access uses a per-directory lock under the Git-ignored `.waap-runtime/` directory. waap must hold the lock across a read, validation, and atomic record update. The lock is released before waiting for an external agent. `waap agent run` persists `running` before launch, records the session ID before running the session, and persists the final status after the session and worktree cleanup.

State commands must not manage Git branches, the index, commits, or remotes, and report no commit hash. Agent execution still creates an isolated Git worktree from the application checkout's HEAD and removes it afterward.

When the primary checkout moves, stop commands and agents before moving a plain state directory. Use `waap check` from the new checkout to identify the destination, create its parent directories, move the old directory, then run `waap check` again. An explicit state path can stay in place. There is no `waap repair` command.

## Verification

- Initialization, validation, and mutations succeed outside any state checkout.
- A user-managed state checkout with `index.lock` present accepts writes without changes to the index or refs. Git ignores the runtime lock.
- Concurrent ticket creations preserve all records, dependency updates cannot publish a cycle, and only one runner can claim an agent.
