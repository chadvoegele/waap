+++
title = "Implement Ticket Get"
creation_date = 2026-06-24T17:59:34Z
status = "completed"
+++

# Spec Reference
Lines 167-173 of /specs/spec.md

# Description
Implement `waap ticket get` so callers can retrieve an existing ticket's metadata and markdown content.

# Requirements
- Add `waap ticket get --ticket-id <ticket-id>`.
- Validate that `--ticket-id` is a valid ticket id.
- Load `.waap/tickets/<ticket-id>/ticket.md`.
- Validate the ticket frontmatter using the same rules as `waap check` and existing ticket operations.
- Print ticket metadata and content to stdout.
- Support both human-readable and JSON output formats.
- Return a non-zero exit code with a useful error message when the ticket id is invalid or the ticket does not exist.
- Preserve the ticket title; ticket ids are not editable.

# Validation
- `cargo fmt`
- `cargo test`
- `cargo run -- check`
