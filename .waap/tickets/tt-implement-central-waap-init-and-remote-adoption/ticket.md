+++
name = "Implement central waap init and remote adoption"
creation_date = 2026-08-02T23:45:59Z
status = "pending"
depends_on = ["tt-add-serialized-central-waap-state-transactions"]
+++

Implement the new setup-only `waap init` behavior from `specs/waap-state-worktree.md` using the project context and Git primitives.

## Scope

- Initialize the derived state worktree or exact `--waap-root` target without changing the application branch or checkout.
- If `origin` is absent or confirmed not to contain `waap`, create the orphan branch, `agents` and `tickets` skeleton, and parentless `waap init` commit.
- If `origin/waap` exists, fetch and adopt verified state-only history automatically.
- Fail before local modification when a configured remote cannot conclusively be queried or fetched.
- Configure `origin/waap` as upstream without pushing.
- Keep initialization setup-only: reject existing selected state and, without an override, legacy `.waap`; never migrate or repair from `init`.
- Include the absolute `state_directory` in human and JSON reports.
- Test fresh local/remote initialization, exact override targets, remote failures, conflicts, idempotent failure, and unchanged application HEAD/worktree.

Use only isolated fixtures. Do not initialize or repair this repository with the newly built binary.
