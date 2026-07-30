+++
name = "Fix Codex stale model cache handling"
creation_date = 2026-07-30T15:25:40Z
status = "ready"
+++

Your role is to implement `.waap/tickets/tt-handle-incompatible-codex-model-cache-schema/ticket.md`.

Follow `.agents/skills/waap/roles/developer/agent.md` completely except for its merge-to-main instructions: this task must remain on your isolated feature branch for review. Do not merge into `main`. Your agent id will be assigned by WAAP; use the worktree and branch it creates.

Requirements:

1. Reproduce and root-cause the reported Codex CLI 0.144.5 model-cache schema error as far as practical. Distinguish a harmless upstream warning from a WAAP startup failure.
2. Implement only a safe, targeted WAAP-side fix. Never delete or rewrite credentials, config, sessions, or unrelated Codex state, and do not globally suppress Codex stderr.
3. Maintain `.waap/agents/<your-agent-id>/work_log.md` with reproduction evidence and rationale.
4. Run every validation in `AGENTS.md`; run `cargo test` outside any command sandbox.
5. Commit changes with both your agent id and ticket id in the message.
6. Fetch and rebase onto current `origin/main`, push your feature branch to `origin`, and open a GitHub pull request against `main` using `gh pr create`. Do not merge it. Include the reproduction, root cause, fix, and validation results in the PR body.
7. Mark the ticket completed only after the branch is pushed and the PR is open. Exit successfully so WAAP records completion.

GitHub authentication is supplied in the environment. Preserve unrelated user file `IDEAS.md` and any concurrent work.
