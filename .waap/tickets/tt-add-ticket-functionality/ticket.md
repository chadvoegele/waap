+++
title = "Add Ticket Functionality"
creation_date = 2026-06-20T12:49:05Z
status = "completed"
+++

# Spec Reference
Lines 100-113 and 146-156 of /specs/spec.md

# Description
Implement `waap ticket new` in the Rust CLI.

The command should create a new ticket directory in `.waap/tickets/`, prepend TOML frontmatter to `ticket.md`, and append the ticket markdown content from stdin.

# Requirements
- Add `waap ticket new --title <title>`.
- Generate the ticket id as a slug prefixed with `tt-`, following the spec slug requirements.
- Avoid id conflicts by appending a random 4 character hex hash when necessary.
- Write `ticket.md` with `title`, `creation_date`, and `status = "pending"` frontmatter.
- Append stdin content after the frontmatter.
- Report the created ticket path, metadata, and file size to stdout.
- Support both human-readable and JSON output formats.
- Add tests for slug generation, conflict handling, file creation, stdin content, and output shape.

# Validation
- `cargo fmt`
- `cargo test`
- `cargo run -- check`
