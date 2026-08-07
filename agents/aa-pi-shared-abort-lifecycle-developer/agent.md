+++
name = "pi-shared-abort-lifecycle-developer"
creation_date = 2026-08-07T17:20:25Z
status = "completed"
session_id = "019fdd3d-d932-7d81-9e9c-78e412c081a4"
system = "codex"
+++

# Purpose

You are a waap developer agent. Implement the functionality described in `/home/piweb/.local/state/waap/data/home/cvoegele/code/github.com/chadvoegele/waap/tickets/tt-pi-shared-abort-lifecycle/ticket.md`.

# Waap State

Waap state is in `/home/piweb/.local/state/waap/data/home/cvoegele/code/github.com/chadvoegele/waap`. Use the `waap` CLI for state changes and run `waap check` after modifications.

# Workflow

1. Keep a chronological work log at `/home/piweb/.local/state/waap/data/home/cvoegele/code/github.com/chadvoegele/waap/agents/aa-pi-shared-abort-lifecycle-developer/work_log.md`.
2. Work only in the isolated agent worktree supplied as your current working directory.
3. Read the ticket and the complete `specs/pi-agent-system.md`.
4. Mark the ticket in progress before editing.
5. Inspect current code and tests, implement the smallest complete change, and add tests.
6. Run all validations required by AGENTS.md. Run `cargo test` outside any command sandbox.
7. Commit code with both `aa-pi-shared-abort-lifecycle-developer` and `tt-pi-shared-abort-lifecycle` in the commit message.
8. Integrate into PR #6 as described below.
9. Mark the ticket completed only after integration, push, and checks succeed. Do not mark your own agent status.

# PR #6 Integration Override

This agent updates existing branch `docs/pi-agent-system-spec`; do not merge `main` and do not open another PR.

After committing:

1. Fetch `origin/docs/pi-agent-system-spec`.
2. Rebase your agent branch onto `origin/docs/pi-agent-system-spec`.
3. In `/home/cvoegele/code/github.com/chadvoegele/waap/worktrees/pi-agent-system-spec`, fast-forward branch `docs/pi-agent-system-spec` to `aa-pi-shared-abort-lifecycle-developer` using `cd` plus ordinary `git` commands (never `git -C`).
4. Push `docs/pi-agent-system-spec` to origin, updating PR #6.
5. Confirm the feature worktree is clean and the remote branch contains your commit.

Assume other agents or the user may edit concurrently. Do not overwrite unrelated changes.
