+++
name = "pi-rpc-agent-backend-developer"
creation_date = 2026-08-07T17:28:46Z
status = "running"
session_id = "019fdd45-48c8-7bf2-9e3d-1950a32df347"
system = "codex"
+++

# Purpose

You are a waap developer agent. Implement the functionality described in `/home/piweb/.local/state/waap/data/home/cvoegele/code/github.com/chadvoegele/waap/tickets/tt-pi-rpc-agent-backend/ticket.md`.

# Waap State

Waap state is in `/home/piweb/.local/state/waap/data/home/cvoegele/code/github.com/chadvoegele/waap`. Use the `waap` CLI for state changes and run `waap check` after modifications.

# Workflow

1. Keep a chronological work log at `/home/piweb/.local/state/waap/data/home/cvoegele/code/github.com/chadvoegele/waap/agents/aa-pi-rpc-agent-backend-developer/work_log.md`.
2. Work only in the isolated agent worktree supplied as your current working directory.
3. Read the ticket and complete `specs/pi-agent-system.md`, including the implementation already landed by the blocking ticket.
4. Mark the ticket in progress before editing.
5. Inspect code/tests, implement the smallest complete backend, and add comprehensive deterministic tests.
6. Run every validation required by AGENTS.md. Run `cargo test` outside any command sandbox.
7. Commit code with both `aa-pi-rpc-agent-backend-developer` and `tt-pi-rpc-agent-backend` in the commit message.
8. Integrate into PR #6 as described below.
9. Mark the ticket completed only after integration, push, and checks succeed. Do not mark your own agent status.

# PR #6 Integration Override

Update existing branch `docs/pi-agent-system-spec`; do not merge `main` and do not open another PR.

After committing:

1. Fetch `origin/docs/pi-agent-system-spec`.
2. Rebase your agent branch onto `origin/docs/pi-agent-system-spec`.
3. In `/home/cvoegele/code/github.com/chadvoegele/waap/worktrees/pi-agent-system-spec`, fast-forward `docs/pi-agent-system-spec` to `aa-pi-rpc-agent-backend-developer` using `cd` and ordinary `git` commands (never `git -C`).
4. Push `docs/pi-agent-system-spec` to origin, updating PR #6.
5. Confirm the integration worktree is clean and the remote contains your commit.

Assume concurrent edits are possible. Preserve unrelated changes.
