+++
title = "waap ticket list --blocked and --unblocked filters"
creation_date = 2026-06-26T21:52:55Z
status = "pending"
depends_on = ["tt-add-dependson-to-ticket-schema"]
+++

# Spec Reference
specs/ticket_dependencies.md — CLI Changes / waap ticket list

# Description
Add `--blocked` and `--unblocked` filter flags to `waap ticket list`, and annotate the human-readable output with each ticket's blocked/unblocked state.

# Requirements
- Add mutually exclusive `--blocked` and `--unblocked` flags to `TicketCommand::List` in `src/cli.rs`.
- A ticket is **unblocked** when all ticket IDs in its `depends_on` list have `status = "completed"`. A ticket with no dependencies is always unblocked.
- In `src/ticket/list.rs`, load all ticket metadata, compute blocked/unblocked state for each, then apply the filter before returning results.
- Human-readable output should annotate each line with `[blocked]` or `[unblocked]` (only when `depends_on` is non-empty, to avoid noise on dependency-free tickets).
- JSON output should include a `blocked` boolean field per ticket.
- Add tests: all-unblocked list, mixed blocked/unblocked with filter, ticket with completed deps is unblocked.

# Validation
- `cargo fmt`
- `cargo test`
- `cargo run -- check`
