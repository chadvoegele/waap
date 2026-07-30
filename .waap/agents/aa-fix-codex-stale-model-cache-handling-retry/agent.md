+++
name = "Fix Codex stale model cache handling retry"
creation_date = 2026-07-30T15:26:14Z
status = "ready"
+++

Recover and complete `.waap/tickets/tt-handle-incompatible-codex-model-cache-schema/ticket.md` after agent `aa-fix-codex-stale-model-cache-handling` could not start because its OpenCode server was unavailable.

Follow `.agents/skills/waap/roles/developer/agent.md` completely except for merge-to-main instructions: keep all implementation on your WAAP-created feature branch and do not merge `main`.

Requirements:

1. Reproduce and root-cause the Codex CLI 0.144.5 `failed to renew cache TTL: missing field supports_reasoning_summaries` error as far as practical, using the actual app-server request flow.
2. Implement the smallest safe, targeted WAAP-side fix. Do not delete or rewrite credentials, config, sessions, or unrelated Codex state; do not globally suppress stderr.
3. Add regression tests for normal startup and the stale-cache/error path. Preserve useful unrelated Codex diagnostics.
4. Maintain `.waap/agents/<your-agent-id>/work_log.md` with evidence and rationale.
5. Run `waap check` and every validation in `AGENTS.md`; run `cargo test` outside a command sandbox.
6. Commit with your agent id and `tt-handle-incompatible-codex-model-cache-schema` in the message.
7. Fetch and rebase onto current `origin/main`, push this feature branch, and open a GitHub PR against `main` with reproduction, root cause, fix, and validations. Do not merge it.
8. Mark the ticket completed only after the PR is open, then exit successfully.

GitHub authentication is supplied in the environment. Preserve unrelated `IDEAS.md` and concurrent work.
