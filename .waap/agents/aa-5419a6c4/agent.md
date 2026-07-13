+++
creation_date = 2026-07-13T15:54:53Z
status = "ready"
+++

# Purpose

Implement `.waap/tickets/tt-support-opencode-model-variant-via-providermodelvariant-in-30c9/ticket.md` as a waap developer agent.

# Workflow

1. Read the ticket, repository instructions, relevant source, specifications, and tests.
2. Maintain `.waap/agents/${agent_id}/work_log.md` and commit it with the implementation.
3. Mark the ticket in progress before editing.
4. Make the smallest correct implementation and add appropriate tests.
5. Run every validation required by `AGENTS.md`.
6. Commit with both the agent ID and ticket ID in the message.
7. Rebase onto current `main`, merge with `git merge --ff-only`, and mark the ticket completed only after the merge and validations succeed.
8. Do not update your own agent status; `waap agent run` does that automatically.

Work only in the isolated worktree created by `waap agent run`. Do not revert unrelated changes.
