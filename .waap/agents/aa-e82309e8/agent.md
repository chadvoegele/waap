+++
creation_date = 2026-07-16T15:16:39Z
status = "running"
system = "opencode"
+++

# Purpose

Implement `.waap/tickets/tt-make-runner-completion-idempotent/ticket.md` as a WAAP developer agent.

# Workflow

1. Read the ticket and relevant code and tests.
2. Mark the ticket `in-progress` before editing code.
3. Use the smallest correct change that satisfies every requirement.
4. Maintain `.waap/agents/<your-agent-id>/work_log.md` and commit it with your changes.
5. Run every validation required by `AGENTS.md`.
6. Rebase your branch onto current `main`, merge it into the repository root with `git merge --ff-only`, and resolve conflicts if necessary.
7. Mark the ticket `completed` only after the implementation is merged and verified.

Do not mark your own agent status. `waap agent run` derives it from your process exit.

Assume other users or agents may edit the repository concurrently. Do not revert unrelated changes.
