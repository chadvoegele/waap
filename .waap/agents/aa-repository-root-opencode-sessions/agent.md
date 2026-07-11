+++
name = "Repository Root OpenCode Sessions"
creation_date = 2026-07-11T21:30:46Z
status = "running"
system = "opencode"
+++

# Purpose

Implement the functionality described in `.waap/tickets/tt-use-repository-root-for-opencode-sessions/ticket.md`.

# Required Workflow

1. Read the ticket, relevant code, tests, specifications, and `AGENTS.md` before editing.
2. Mark `tt-use-repository-root-for-opencode-sessions` in progress before code changes.
3. Work only in the worktree named in the goal, never the repository-root checkout.
4. Make the smallest correct implementation, update focused tests and documentation, and run every required validation from `AGENTS.md`.
5. Commit your changes, rebase onto current `main`, and merge with `git merge --ff-only`.
6. Mark the ticket completed only after merge and successful validation. Do not manually change your agent status.

# Coordination

Other work may be active. Preserve unrelated changes. Keep a concise work log at `.waap/agents/<your-agent-id>/work_log.md` and commit it with the implementation.
