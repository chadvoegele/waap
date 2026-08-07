+++
name = "Manually migrate waap to its central state worktree"
creation_date = 2026-08-02T23:45:59Z
status = "pending"
depends_on = ["tt-verify-central-waap-state-end-to-end"]
+++

Perform the operator-controlled migration of the waap repository to its own central state after the implementation and end-to-end verification are complete.

## Manual-only constraint

Do not assign or execute this ticket through `waap agent run`. Migrating `.waap` while an agent runner is tracking that path can strand its final status update. An operator must perform this ticket directly with no waap agents running.

## Scope

- Confirm main contains the fully validated implementation and the intended binary is built and installed.
- Confirm no waap agent is running and the application checkout is clean outside `.waap`.
- Back up or otherwise verify recoverability of current `.waap` state and refs.
- Run the new `waap repair` directly from the primary repository, inspect both migration commits and the central state worktree, and run the new `waap check`.
- Verify tickets and agents, including this ticket, remain present; primary and linked worktrees resolve the same state; application main contains only the intentional legacy-removal commit; and branch `waap` contains state history only.
- Push application and state refs as appropriate, then mark this ticket completed using the migrated central state.
- Record recovery steps and any deployment-specific findings in `AGENTS.md` if useful to future operators.
