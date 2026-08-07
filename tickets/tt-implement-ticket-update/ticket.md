+++
title = "Implement Ticket Update"
creation_date = 2026-06-23T12:13:27Z
status = "completed"
+++

# Spec Reference
Lines 157-166 of /specs/spec.md

# Description
Implement the `waap ticket update` CLI command so existing ticket frontmatter can be updated without changing the ticket id or markdown body content.

# Requirements
- Add `waap ticket update --ticket-id <ticket-id> --set-status <status>`.
- Validate the ticket id format and that `.waap/tickets/<ticket-id>/ticket.md` exists.
- Accept only valid ticket statuses: `pending`, `in-progress`, `completed`, and `abandoned`.
- Preserve the existing ticket `title`, `creation_date`, and markdown body while updating only requested metadata.
- Report the updated ticket path, metadata, and file size to stdout.
- Support both human-readable and JSON output formats.
- Add tests for argument parsing, missing tickets, invalid statuses, frontmatter preservation, body preservation, file size reporting, and output shape where practical.

# Validation
- `cargo fmt`
- `cargo test`
- `cargo run -- check`
