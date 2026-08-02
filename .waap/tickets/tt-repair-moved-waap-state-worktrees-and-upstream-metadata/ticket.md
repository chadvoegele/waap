+++
name = "Repair moved waap state worktrees and upstream metadata"
creation_date = 2026-08-02T23:45:59Z
status = "pending"
depends_on = ["tt-implement-legacy-waap-state-migration-repair"]
+++

Extend `waap repair` with repository relocation and upstream metadata recovery from `specs/waap-state-worktree.md`.

## Scope

- Detect a moved primary repository by comparing the newly derived state path with the registered `waap` worktree path in the common Git directory.
- From the moved primary checkout, run the required `git worktree repair`, move the state worktree to the new derived path, and repair registration/back-links again.
- Preserve branch history, state contents, staged/unstaged/untracked files, and application branches.
- Fail before moving anything when the destination is occupied or the registered state worktree cannot be identified uniquely and safely.
- Repair missing or incorrect `origin/waap` upstream configuration, including when `origin` was added after initialization.
- Keep repair idempotent and provide old path, expected path, and recovery guidance in diagnostics.
- Test real directory relocation, broken back-links, occupied destinations, dirty state preservation, invocation from unsupported linked worktrees, and upstream repair.

Use disposable repositories; never move this repository's actual state during implementation.
