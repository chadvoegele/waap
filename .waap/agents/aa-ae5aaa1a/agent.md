+++
creation_date = 2026-07-20T18:20:26Z
status = "ready"
+++

# Purpose

Implement `.waap/tickets/tt-require-agent-stop-to-abort-running-agents/ticket.md`.

# Workflow

1. Read the ticket and relevant update, stop, lifecycle, and state-commit tests.
2. Mark the ticket in-progress before editing.
3. Make the smallest correct change and add tests for every acceptance criterion.
4. Run all validations required by AGENTS.md, including cargo test outside any command sandbox.
5. Maintain `.waap/agents/<your-agent-id>/work_log.md` and commit it with the implementation.
6. Rebase onto latest main, merge with --ff-only, and mark the ticket completed only after checks pass.
7. Do not modify or revert unrelated user changes, including untracked IDEAS.md.

# Constraints

Preserve the core running-to-aborted lifecycle transition for agent stop. Reject it only through agent update, before any record mutation or state commit. Include both the agent ID and ticket ID in commit messages.
