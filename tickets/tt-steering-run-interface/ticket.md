+++
name = "Steering run interface"
creation_date = 2026-10-20T13:56:15Z
status = "pending"
+++

# Steering run interface

## Purpose
Make the active run steerable without coupling shared orchestration to a backend.

## Scope
- Inspect current `RunHandle::wait`, shared run lifecycle, Codex blocking reader,
  Claude process wait, and OpenCode SSE handling before choosing concurrency.
- Establish a small active-run interface that supports steering while completion,
  stop, and output handling continue. Shared orchestration must not block solely
  inside the existing consuming wait. Avoid a speculative framework or unnecessary
  async-runtime dependency; explain the chosen ownership/event-loop boundary in
  the work log for following agents.
- Add backend-neutral accepted/rejected/unknown error contracts as needed.
- Claude and OpenCode must return not implemented from their backend run steering
  implementations, not the CLI or shared run layer. Codex may temporarily return
  not implemented until its dependent ticket, but remove that placeholder there.
- Preserve existing successful/failed/aborted lifecycle, stop semantics, streaming
  output, child ownership, and worktree cleanup, including start/wait failures.

## Acceptance
- Unit/fake-backed tests exercise steering on an active run while completion can
  progress; unsupported behavior originates in Claude/OpenCode implementations.
- Existing lifecycle/backend tests pass, and the feature branch compiles.
- Document the exact interface handoff for the socket and Codex tickets.

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
