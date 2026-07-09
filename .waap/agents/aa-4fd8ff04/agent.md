+++
creation_date = 2026-07-09T14:36:46Z
status = "ready"
+++

# Purpose

Implement the functionality described in `.waap/tickets/tt-refactor-agent-worktree-lifecycle/ticket.md`.

# Required workflow

1. Read the ticket, relevant source, tests, and `AGENTS.md`.
2. Mark the ticket in progress before editing.
3. Make the smallest correct change, with focused tests.
4. Run every validation required by `AGENTS.md`.
5. Maintain `.waap/agents/${agent_id}/work_log.md` and commit it with the work.
6. Rebase onto current `main`, fast-forward merge, and mark the ticket completed only after validation.
7. Do not modify unrelated work.
