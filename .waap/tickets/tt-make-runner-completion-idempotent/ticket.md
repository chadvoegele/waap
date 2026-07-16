+++
name = "Make runner completion idempotent"
creation_date = 2026-07-16T15:07:10Z
status = "completed"
+++

# Problem

`waap agent run` treats completion as a strict `running -> completed` transition. If the running agent executes `waap agent update --set-status completed`, the backend exits successfully but the runner then attempts `completed -> completed`. That transition fails, and the runner's attempt to persist a failed run also fails with `completed -> failed`.

The work performed by the agent succeeded, so the runner must not report this case as a failure.

Runner-owned failure persistence has the same problem. If an agent marks itself `failed` before returning a failed backend outcome or runner error, the runner attempts `failed -> failed`. This can replace the backend exit code or obscure the primary error with a secondary persistence error.

# Requirements

- Make runner-owned terminal persistence idempotent: before marking the agent `completed` or `failed`, read its current persisted status and return without another state change or commit when it already equals the desired status.
- Keep the general agent transition graph and explicit `waap agent update` behavior strict unless a broader change is justified.
- Keep `ready -> running` and explicit `waap agent stop` behavior strict.
- Continue rejecting conflicting terminal states. Successful backend completion must not overwrite `aborted` or `failed`, and failure persistence must not overwrite `aborted` or `completed`.
- Preserve the backend exit code when a failed backend has already persisted the agent as `failed`.
- Preserve the primary runner error without a secondary failure-persistence error when the agent is already `failed`.
- Avoid producing empty duplicate terminal-state commits.
- Add regression tests covering:
  - a successful run whose agent already marked itself `completed`;
  - direct idempotent runner completion;
  - a failed backend whose agent already marked itself `failed`;
  - a runner error after the agent marked itself `failed`;
  - direct idempotent runner failure persistence; and
  - rejection of conflicting terminal states.
- Run all validations required by `AGENTS.md`.

# Acceptance Criteria

Runner-owned persistence of an already-matching `completed` or `failed` status succeeds without changing the agent file or creating another commit. A successful backend whose agent already marked itself `completed` exits successfully. A failed backend whose agent already marked itself `failed` returns its original exit code. A runner error whose agent already marked itself `failed` preserves its primary error without a secondary persistence error. Explicit lifecycle operations and conflicting terminal states remain strict.
