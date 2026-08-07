+++
name = "Implement central waap check and remote freshness validation"
creation_date = 2026-08-02T23:45:59Z
status = "in-progress"
depends_on = ["tt-repair-moved-waap-state-worktrees-and-upstream-metadata"]
+++

Implement central-state `waap check` behavior from `specs/waap-state-worktree.md`.

## Scope

- Always report the resolved absolute state directory, including absent, legacy-only, coexistence, and relocation-error cases; add `state_directory` to JSON.
- Without `--waap-root`, require repair for legacy-only state and fail when central and legacy state coexist. With an override, inspect only the supplied state directory.
- Validate branch, upstream configuration, expected worktree registration/path, root directories, state-only history reachable from `waap`, record schemas, IDs, statuses, dependencies, and current file contents.
- Do not inspect or compare non-`waap` refs.
- Fail for staged, unstaged, or untracked state-worktree changes and list affected paths.
- Fetch `origin/waap` when possible. Accept no `origin` or a confirmed missing remote branch; warn on query/fetch failure and on remote-only/diverged commits without changing an otherwise successful result.
- Keep warnings on stderr and JSON stdout valid.
- Add comprehensive tests for all invariants, overrides, warnings, dirty states, and malformed/conflicting history.

Do not point the newly built checker at this repository's live legacy state except in a controlled, non-mutating diagnostic explicitly designed for the cutover.
