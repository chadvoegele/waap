+++
title = "Implement Ticket List"
creation_date = 2026-06-23T19:13:12Z
status = "completed"
+++

# Spec Reference
Lines 174-180 of /specs/spec.md

# Description
Implement `waap ticket list` so callers can list existing ticket ids from `.waap/tickets/`, optionally filtering by ticket status.

# Requirements
- Add `waap ticket list`.
- Add optional `--status <status>` filtering.
- Accept only valid ticket statuses: `pending`, `in-progress`, `completed`, and `abandoned`.
- Validate existing ticket frontmatter while listing, reusing the same schema expectations as `waap check` where practical.
- Return ticket ids on stdout.
- Support both human-readable and JSON output formats.
- Add tests for argument parsing, status filtering, invalid status handling, empty ticket directories, and output shape where practical.

# Validation
- `cargo fmt`
- `cargo test`
- `cargo run -- check`
