+++
name = "Steering integration verification"
creation_date = 2026-10-20T13:56:16Z
status = "pending"
depends_on = ["tt-steering-cli-client"]
+++

# Steering integration verification

## Purpose
Audit the integrated feature against every requirement and fix gaps before live testing.

## Scope
- Review code against `specs/agent-steering.md`, SVG, and the agents repository's
  CODE.md. Prefer a small composable design with explicit ownership, minimal
  abstractions, named limits, focused helpers, and no dead placeholders.
- Extend automated integration coverage using a fake Codex app server and actual
  run/steer CLI processes: stream output, steer the active turn, complete, verify
  original worktree lifecycle, reject a later steering call, and verify stop.
- Cover queue/connection/frame boundaries, deadline before vs after dispatch,
  active-dispatch timeout and late acknowledgements, pending exit rejection,
  same-repo worktree lookup, separate repositories, privacy and path limits.
- Assert exact native request and no extra turn/session; exercise Claude/OpenCode
  unsupported errors through their backend implementations. Do not require live
  external backend credentials for the automated suite.
- Confirm no persistent state schema changes or steering state commits.
- Run all five required validations, repair failures, record commands/results in
  the work log, and supply a concise reproducible live-test procedure for next ticket.

## Acceptance
- All specification verification items are covered, with any unavoidable coverage
  limitations explicitly recorded rather than represented as passed.
- Full formatting, clippy, debug build, release build, and test suite pass.
- Changes are integrated into the single feature branch; no PR yet.

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
