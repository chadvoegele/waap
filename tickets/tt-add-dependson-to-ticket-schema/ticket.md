+++
title = "Add depends_on to ticket schema"
creation_date = 2026-06-26T21:52:23Z
status = "completed"
+++

# Spec Reference
specs/ticket_dependencies.md

# Description
Add a `depends_on` field to `TicketMetadata` so tickets can declare structured dependencies on other tickets.

# Requirements
- Add `depends_on: Option<Vec<String>>` to `TicketMetadata` in `src/ticket.rs`.
- Parse it in `TicketMetadata::from_frontmatter()`: optional array of strings. Missing or empty → `None` (or empty vec).
- Add a `require_optional_string_array()` validator to `src/frontmatter.rs` that validates each element is a string.
- Serialize it in `TicketMetadata::to_frontmatter_lines()`: emit the field only when non-empty.
- Expose `depends_on` in `TicketReport` and include it in both human-readable and JSON output of `waap ticket get`.
- Add unit tests for parse/serialize round-trip.

# Validation
- `cargo fmt`
- `cargo test`
- `cargo run -- check`
