+++
name = "Steering live Codex and PR"
creation_date = 2026-10-20T13:56:16Z
status = "pending"
depends_on = ["tt-steering-integration-verification"]
+++

# Steering live Codex and PR

## Purpose
Perform a real Codex smoke test, then open the single implementation PR for Chad.

## Scope
- Build/use the feature branch's binary explicitly (not the older installed waap).
- Use a disposable short-path Git repository and private runtime directory with
  valid XDG_RUNTIME_DIR ownership/mode. Initialize its isolated waap state using
  that binary. Do not mutate production code or existing user agents during smoke.
- Create a one-shot real Codex test agent that remains active long enough to steer
  and can demonstrate a unique steering-only marker in its final response or an
  allowed disposable artifact. Use normal native app-server auth; never expose
  credentials or use a fake app server as a substitute for this live test.
- Launch through the new binary; send `agent steer` while its turn is active.
  Verify both command acknowledgement and observable use of the steering-only
  marker, original session continuity, successful completion, socket cleanup,
  and non-running rejection after completion. Check a real stop/cleanup smoke
  where practical. Do not misrepresent acknowledgement alone as agent compliance.
- Record exact binary/version, Codex version, commands, exit statuses, acceptance
  and behavioral evidence, state/worktree/socket outcomes. Keep secrets out of
  logs. Clean up only test-created resources.
- If authentication, model access, runtime constraints, or native API incompatibility
  blocks the smoke, record the precise blocker and report it; never claim a pass.
  Fix implementation bugs on the same feature branch and rerun checks/live smoke.
- Run all five required repository checks again after fixes and review final diff.
- Push `feature-agent-steering`; search for an existing PR for that head first.
  Open/update exactly ONE GitHub PR from this branch to main. Title it clearly,
  include scope, tests, live evidence/limitations, and mark human-facing text as
  "Chad's Agent". Request Chad's review (self-review restrictions may prevent
  formal reviewer assignment; note that clearly rather than requesting a stranger).
- DO NOT merge to main. Await Chad's review and explicit approval.

## Acceptance
- Real Codex steering smoke is passed with recorded evidence, or ticket stays
  incomplete with a precise blocker; no fake success.
- Full checks pass and the single implementation PR contains all ticket changes.
- Work log records PR URL and validation results; main remains unchanged.

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
