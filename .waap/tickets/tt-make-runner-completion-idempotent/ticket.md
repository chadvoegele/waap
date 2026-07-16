+++
name = "Make runner completion idempotent"
creation_date = 2026-07-16T15:07:10Z
status = "pending"
+++

# Problem

`waap agent run` treats completion as a strict `running -> completed` transition. If the running agent executes `waap agent update --set-status completed`, the backend exits successfully but the runner then attempts `completed -> completed`. That transition fails, and the runner's attempt to persist a failed run also fails with `completed -> failed`.

The work performed by the agent succeeded, so the runner must not report this case as a failure.

# Requirements

- Make runner-owned agent completion idempotent: before marking the agent completed, read its current persisted status and return success without another state change or commit when it is already `completed`.
- Keep the general agent transition graph and explicit `waap agent update` behavior strict unless a broader change is justified.
- Continue rejecting conflicting terminal states such as `aborted` or `failed`; successful backend completion must not overwrite them.
- Avoid producing an empty duplicate completion commit.
- Add regression tests covering a successful run whose agent already marked itself completed and direct idempotent runner completion.
- Run all validations required by `AGENTS.md`.

# Acceptance Criteria

A successful `waap agent run` exits successfully and leaves the agent `completed` when the agent process already transitioned itself to `completed`. The output must not contain `invalid agent status transition: completed -> completed` or a secondary `completed -> failed` persistence error.
