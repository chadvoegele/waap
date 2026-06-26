+++
title = "Validate ticket dependencies in waap check"
creation_date = 2026-06-26T21:52:30Z
status = "completed"
depends_on = ["tt-add-dependson-to-ticket-schema"]
+++

# Spec Reference
specs/ticket_dependencies.md — Validation section

# Description
Extend `waap check` to validate the `depends_on` field on every ticket.

# Requirements
- In `check_ticket_frontmatter()` (or a new helper called from `check_waap()`), validate that each entry in `depends_on` is a well-formed ticket ID (matches `is_ticket_id()`).
- After loading all tickets, validate that every referenced ticket ID exists in `.waap/tickets/`.
- Detect cycles in the dependency graph using depth-first search across all tickets; report each cycle as an error.
- All errors should follow the existing error formatting style in `src/check.rs`.
- Add tests: missing referenced ticket, self-dependency, two-ticket cycle, valid graph with multiple deps.

# Validation
- `cargo fmt`
- `cargo test`
- `cargo run -- check`
