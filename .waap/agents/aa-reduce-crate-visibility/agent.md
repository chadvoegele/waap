+++
name = "reduce-crate-visibility"
creation_date = 2026-07-08T14:36:38Z
status = "ready"
+++

# Purpose

Implement `.waap/tickets/tt-reduce-unnecessary-crate-visibility/ticket.md` as WAAP developer agent `aa-reduce-crate-visibility`.

# Workflow

1. Read the ticket, referenced specifications, repository instructions, and current code.
2. Mark the ticket `in-progress` before editing.
3. Audit the current post-dependency code; if earlier tickets removed or renamed an item, audit its replacement rather than restoring it.
4. Make the smallest correct visibility cleanup and update tests/specifications when needed.
5. Maintain `.waap/agents/aa-reduce-crate-visibility/work_log.md` and commit it with the implementation.
6. Run `cargo clippy --all-targets -- -D warnings`, `cargo fmt --check`, `cargo build`, `cargo build --release`, and `cargo test`; run tests outside the command sandbox as required.
7. Run `waap check`.
8. Commit with both `aa-reduce-crate-visibility` and `tt-reduce-unnecessary-crate-visibility` in the message.
9. Rebase onto current `main`, rerun validation, and fast-forward merge with `git merge --ff-only`.
10. Mark the ticket `completed` only after the implementation is merged and checks pass. Do not mark the agent status; `waap agent run` does that automatically.

Preserve unrelated work, including the existing untracked `IDEAS.md`. The worktree is managed by `waap agent run`; do not create or remove it yourself.
