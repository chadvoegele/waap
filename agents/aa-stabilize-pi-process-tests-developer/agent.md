+++
name = "stabilize-pi-process-tests-developer"
creation_date = 2026-08-07T17:58:28Z
status = "completed"
session_id = "019fdd60-76ce-71e0-8644-b09f00cc486b"
system = "codex"
+++

# Purpose

You are a waap developer agent. Implement and verify `/home/piweb/.local/state/waap/data/home/cvoegele/code/github.com/chadvoegele/waap/tickets/tt-stabilize-pi-process-tests/ticket.md`.

# Workflow

Use the supplied isolated worktree. Read the ticket, `specs/pi-agent-system.md`, and current Pi process code/tests. Mark the ticket in progress, reproduce the flake repeatedly, fix its root cause without assertion weakening or masking sleeps, and run all AGENTS.md validations with `cargo test` outside any command sandbox. Maintain `/home/piweb/.local/state/waap/data/home/cvoegele/code/github.com/chadvoegele/waap/agents/aa-stabilize-pi-process-tests-developer/work_log.md`. Commit with both `aa-stabilize-pi-process-tests-developer` and `tt-stabilize-pi-process-tests` in the message. Mark the ticket completed only after integration and checks; do not mark your own agent status.

# PR #6 Integration Override

Update `docs/pi-agent-system-spec` and PR #6 only; do not merge main or open another PR. Fetch and rebase onto `origin/docs/pi-agent-system-spec`, then in `/home/cvoegele/code/github.com/chadvoegele/waap/worktrees/pi-agent-system-spec` fast-forward the feature branch to `aa-stabilize-pi-process-tests-developer` using `cd` and ordinary `git` commands, never `git -C`. Push the feature branch and verify the remote and PR checks.
