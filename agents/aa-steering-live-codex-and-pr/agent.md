+++
name = "Steering live Codex and PR"
creation_date = 2026-10-20T13:56:16Z
status = "ready"
+++

# Waap developer: Steering live Codex and PR

Implement `/home/chad/.local/state/waap/data/home/chad/code/github.com/chadvoegele/waap/tickets/tt-steering-live-codex-and-pr/ticket.md`.
First read that ticket, `specs/agent-steering.md`, repository AGENTS.md, and
`/home/chad/code/code.voegele.me/chad/agents/main/CODE.md`. You are a waap developer agent. Follow the developer-role
workflow at `/home/chad/.pi/agent/skills/waap/roles/developer/agent.md`, EXCEPT
its merge-to-main instructions are explicitly overridden by this ticket's
feature-branch handoff. Never merge to main or create per-ticket PRs.

Run `waap check` to confirm state paths. Before editing in the launcher-created
worktree, rebase your branch onto `feature-agent-steering`. Integrate only into
`/home/chad/code/github.com/chadvoegele/waap/feature-agent-steering` and push `feature-agent-steering`. Keep work logs, tests, and ticket
status accurate. Only the final live-test/PR agent creates or updates the single
implementation PR. Do not launch other agents or execute blocked tickets.

## Shared requirements

Authoritative specification: `specs/agent-steering.md` and its SVG, merged in PR #8.
Read and follow `/home/chad/code/code.voegele.me/chad/agents/main/CODE.md` and repository `AGENTS.md` before coding.
This ticket belongs to the single steering implementation PR. Integrate only into
`feature-agent-steering`, checked out at `/home/chad/code/github.com/chadvoegele/waap/feature-agent-steering`. Never merge to main, open a
per-ticket PR, or create additional implementation PRs. The final ticket owns PR
creation. Preserve unrelated changes. Do not expand scope to support steering in
Claude/OpenCode; their backend methods return not implemented. Shared layers are
backend-neutral. Do not introduce persisted waap metadata or tracked runtime files.

Use waap developer workflow, except the feature-branch integration instructions
below override any developer-role instruction to merge to main. Mark the ticket
in-progress before coding, and completed only after checks and feature integration.
Keep an agent work log in the dedicated waap state directory. Do not mark your own
agent completed; the launcher does that. Run `waap check` after state changes.

Required final checks: `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`,
`cargo build`, `cargo build --release`, `cargo test` (outside a command sandbox).
Use focused tests during development and keep the feature branch compiling.

## Branch handoff

The launcher creates your isolated agent worktree from the canonical checkout.
Before editing, rebase your agent branch onto local `feature-agent-steering` to
pick up prior tickets. Do all implementation work in your launcher-created worktree.
Commit with agent ID and ticket ID. Before integrating, rebase again onto
`feature-agent-steering`, resolve conflicts, and rerun affected checks. Then run
`git -C /home/chad/code/github.com/chadvoegele/waap/feature-agent-steering merge --ff-only <your-agent-branch>` and push that feature branch
to origin. Do not remove your worktree; the launcher owns cleanup. Sequential
execution is intentional because these tickets change the same lifecycle/protocol
interfaces. Ticket completion refers to integration into the feature branch, NOT
approval or merge of the final PR.

