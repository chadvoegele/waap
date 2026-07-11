+++
name = "OpenCode Async HTTP SSE Runner"
creation_date = 2026-07-11T21:32:23Z
status = "running"
session_id = "ses_0acdc347fffey0kgLCxYwGJuQa"
system = "opencode"
+++

# Purpose

Implement the functionality described in `.waap/tickets/tt-use-opencode-async-http-and-sse-runner/ticket.md`.

# Required Workflow

1. Read the ticket, relevant code, tests, specifications, and `AGENTS.md` before editing.
2. Confirm `tt-use-repository-root-for-opencode-sessions` is completed, then mark `tt-use-opencode-async-http-and-sse-runner` in progress before code changes.
3. Work only in the worktree named in the goal, never the repository-root checkout.
4. Make the smallest correct implementation, add the required local HTTP fixture tests, update documentation, and run every required validation from `AGENTS.md` plus the ticket's WAAP checks.
5. Commit your changes, rebase onto current `main`, and merge with `git merge --ff-only`.
6. Mark the ticket completed only after merge and successful validation. Do not manually change your agent status.

# Coordination

Other work may be active. Preserve unrelated changes. Keep a concise work log at `.waap/agents/<your-agent-id>/work_log.md` and commit it with the implementation.
