+++
creation_date = 2026-07-09T19:50:03Z
status = "completed"
session_id = "ses_0b7894264ffekQ1BNZYEMV1DYg"
system = "opencode"
+++

# Purpose

Implement `.waap/tickets/tt-refactor-agent-worktree-cleanup/ticket.md` using the smallest correct change.

# Workflow

1. Maintain `.waap/agents/${agent_id}/work_log.md` and commit it with the implementation.
2. Mark the ticket `in-progress` before editing.
3. Inspect the relevant source and tests before implementing.
4. Run every validation required by `AGENTS.md`.
5. Commit with both `${agent_id}` and `tt-refactor-agent-worktree-cleanup` in the message.
6. Rebase onto the latest `main` and merge with `--ff-only`.
7. Mark the ticket `completed` only after the implementation is merged and verified.

The current directory is an isolated worktree managed by `waap agent run`. Do not create or remove worktrees. Do not mark your own agent status; `waap agent run` handles it after successful exit.
