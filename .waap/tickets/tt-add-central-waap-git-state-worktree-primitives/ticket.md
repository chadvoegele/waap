+++
name = "Add central waap Git state worktree primitives"
creation_date = 2026-08-02T23:45:59Z
status = "in-progress"
depends_on = ["tt-introduce-central-waap-project-context-resolution"]
+++

Implement reusable Git primitives for the central state branch and worktree described in `specs/waap-state-worktree.md`.

## Scope

- Inspect local refs, worktree registrations, checked-out branches, common-Git metadata, and branch upstream configuration without changing unrelated refs or worktrees.
- Create a local orphan `waap` branch and dedicated state worktree at an exact path.
- Query and fetch `origin/waap`, distinguish a confirmed missing branch from query/fetch failure, and configure `branch.waap.remote` and `branch.waap.merge` without pushing.
- Adopt existing remote state only when every commit reachable from it contains state paths under `agents/` and `tickets/`. Validate only `waap` history; never inspect or compare non-`waap` branches.
- Detect conflicting local `waap` branches, unexpected checkout locations, occupied paths, missing registrations, and state history containing non-state paths without resetting or deleting them.
- Add isolated Git tests for fresh orphan creation, remote adoption, missing/unreachable remotes, state-only history, conflicts, upstream configuration, and application checkout preservation.

Keep these primitives unwired from production dispatch until the activation ticket. Never create or move this repository's real `waap` branch or state worktree during implementation.
