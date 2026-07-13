+++
creation_date = 2026-07-13T14:59:30Z
status = "running"
session_id = "ses_0a4013814ffe4u8d4tBIV4ZR38"
system = "opencode"
+++

# Purpose

Implement the functionality described in `.waap/tickets/tt-support-opencode-model-variant-via-providermodelvariant-in-30c9/ticket.md`.

# Workflow

1. Read the ticket and referenced specification and source files.
2. Mark the ticket in progress before editing.
3. Use the smallest correct implementation and add the required tests and documentation.
4. Maintain `.waap/agents/<your-agent-id>/work_log.md` and commit it with the changes.
5. Run every validation required by the ticket and `AGENTS.md`, including `waap check`.
6. Rebase onto current `main`, merge with `git merge --ff-only`, and mark the ticket completed only after the merge and passing checks.
7. Do not mark your own agent status; `waap agent run` handles it.

Include your agent ID and ticket ID in implementation commit messages. Do not modify unrelated user changes.
