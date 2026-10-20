+++
name = "Steering control socket"
creation_date = 2026-10-20T13:56:15Z
status = "pending"
depends_on = ["tt-steering-run-interface"]
+++

# Steering control socket

## Purpose
Implement the shared, backend-neutral runtime control server.

## Scope
- Implement the symmetric Unix-domain socket path under
  `$XDG_RUNTIME_DIR/waap/control/<repository-relative-path>/agents/<agent-id>.sock`.
  Reuse state repository identity across worktrees. Validate runtime ownership and
  privacy; missing/empty/insecure runtime and overly long socket paths fail clearly
  without fallback. Never track sockets or add persisted metadata.
- Shared agent-run lifecycle owns bind/read/parse/queue/reply/cleanup for ALL
  backends. Delegate to the active run interface from the prerequisite ticket.
- Implement version-1 NDJSON schemas and errors from the spec, one request and one
  response per connection; no handshake, run/session IDs, reconnect, or retries.
- Enforce a 16-pending-plus-one-dispatch FIFO, 64-KiB frame limit including newline,
  32 connection cap, 5-second incomplete-frame deadline, and 30-second server
  acknowledgement deadline including queue time. Use monotonic time.
- Expired queued requests never dispatch; post-dispatch uncertainty is not rejection.
  Handle late acknowledgements without concurrent dispatch or response corruption.
- Reject queued requests on completion and remove the socket on normal exit.
  Scope stale-endpoint handling to the spec rather than adding recovery state.

## Acceptance
- Fake backend and real Unix-socket integration tests cover schemas, malformed
  input, version/method rejection, exact prompt preservation, response correlation,
  FIFO/overflow, limits/deadlines, exit and cleanup, and repository isolation.
- Claude/OpenCode requests reach their backend methods and return not implemented.
- The server progresses alongside backend output and completion without holding
  the waap state lock for the duration of a run.

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
