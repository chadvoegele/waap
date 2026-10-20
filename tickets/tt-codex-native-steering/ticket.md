+++
name = "Codex native steering"
creation_date = 2026-10-20T13:56:15Z
status = "pending"
depends_on = ["tt-steering-control-socket"]
+++

# Codex native steering

## Purpose
Implement native Codex turn steering through the existing live app-server owner.

## Scope
- Inspect the installed Codex app-server schema/version and confirm native
  `turn/steer` request/response fields before implementing. Record version/schema
  evidence; do not substitute turn/start or restart/resume.
- Implement the active backend run interface's steering method using the existing
  thread ID, active expected turn ID, and unchanged text input.
- Refactor request multiplexing as needed so app-server notifications, initial
  turn/start acknowledgement, steering replies, completion, and SIGTERM-driven
  turn/interrupt continue correctly. Single protocol owner; no competing readers.
- Correlate RPC IDs, forward output, reject startup/no-active-turn/completed turn,
  surface server rejection and acknowledgement uncertainty, and handle late replies.
- Missing native method returns unsupported_backend_version with an actionable
  diagnostic. Do not leak Codex thread/turn concepts into the control wire schema.

## Acceptance
- Protocol/fake-server tests cover accepted steering, rejection, missing method,
  exact payload, unrelated notifications, interleaved completion/interrupt,
  startup/completed-turn rejection, EOF, and uncertain delivery without retry.
- Tests prove no extra session/turn is created and original lifecycle is preserved.

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
