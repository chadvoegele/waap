+++
name = "Add serialized central waap state transactions"
creation_date = 2026-08-02T23:45:59Z
status = "in-progress"
depends_on = ["tt-add-central-waap-git-state-worktree-primitives"]
+++

Implement serialized mutation transactions for central waap state as specified in `specs/waap-state-worktree.md`.

## Scope

- Add one per-repository lock outside the state worktree, preferably in the common Git directory.
- Hold the lock across state pre-validation, file changes, post-validation, explicit staging, and commit.
- Commit only requested state paths on branch `waap`; preserve no-op behavior and return the state commit hash.
- Ensure failures and competing writers leave no partially staged, invalid, or accidentally committed state.
- Make each `waap agent run` status transition a separate transaction; do not hold the lock while an external agent runs.
- Refactor mutation APIs so they can receive state and source context without activating central-state dispatch yet.
- Add concurrency, rollback, explicit-path, no-op, and lock-contention tests in isolated repositories.

Do not exercise the new transaction code against this repository's live `.waap` state.
