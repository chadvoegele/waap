+++
name = "Implement legacy waap state migration repair"
creation_date = 2026-08-02T23:45:59Z
status = "in-progress"
depends_on = ["tt-implement-central-waap-init-and-remote-adoption"]
+++

Add `waap repair` and implement safe legacy `.waap` migration from `specs/waap-state-worktree.md`.

## Scope

- Add CLI parsing, dispatch, reports, and actionable errors for `waap repair`.
- Without `--waap-root`, detect legacy-only state and require no conflicts or staged/unstaged changes outside `.waap`; allow dirty legacy state.
- Validate legacy contents, create central orphan state, copy all legacy state, validate it, and commit `waap migrate state` before deleting anything.
- Remove legacy `.waap` only after central state is valid and committed, then commit the application-branch deletion as `Remove legacy waap state`.
- Preserve recoverable central and legacy copies if source cleanup fails; later commands must report coexistence rather than choose or merge.
- Make repair idempotent and fail rather than guess on ambiguous state.
- Ensure `--waap-root` uses only its supplied target and does not inspect legacy state.
- Add failure-injection and integration tests proving ordering, dirty-state handling, no data loss, explicit-root isolation, and unchanged unrelated files.

## Self-hosting safety

Do not run this repair against the waap repository itself from an agent session. Migrating the state that tracks the running agent would invalidate the runner's original state path. Test only in temporary repositories.
