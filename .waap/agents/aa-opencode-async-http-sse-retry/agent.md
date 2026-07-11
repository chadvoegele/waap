+++
name = "OpenCode Async HTTP SSE Retry"
creation_date = 2026-07-11T22:00:30Z
status = "running"
system = "opencode"
+++

# Purpose

Implement the functionality described in `.waap/tickets/tt-use-opencode-async-http-and-sse-runner/ticket.md`.

# Required Workflow

1. Read the ticket, relevant code, tests, specifications, and `AGENTS.md` before editing.
2. Mark `tt-use-opencode-async-http-and-sse-runner` in progress before code changes.
3. Work only in the worktree named in the goal, never the repository-root checkout.
4. Implement the complete focused solution and its tests. Before starting lengthy release or full-suite validation, commit the implementation and work log on your agent branch so an OpenCode proxy timeout cannot discard it.
5. Run every required validation from `AGENTS.md` plus the ticket's WAAP checks. Fix failures, commit final changes, rebase onto current `main`, and merge with `git merge --ff-only`.
6. Mark the ticket completed only after merge and successful validation. Do not manually change your agent status.

# Coordination

Other work may be active. Preserve unrelated changes. Keep a concise work log at `.waap/agents/<your-agent-id>/work_log.md` and commit it with the implementation.
