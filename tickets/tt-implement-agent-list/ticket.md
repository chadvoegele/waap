+++
title = "Implement Agent List"
creation_date = 2026-06-23T19:13:12Z
status = "completed"
+++

# Spec Reference
Lines 225-231 of /specs/spec.md

# Description
Implement `waap agent list` so callers can list existing agent ids from `.waap/agents/`, optionally filtering by agent status.

# Requirements
- Add `waap agent list`.
- Add optional `--status <status>` filtering.
- Accept only valid agent statuses: `ready`, `running`, `completed`, and `aborted`.
- Validate existing agent frontmatter while listing, reusing the same schema expectations as `waap check` where practical.
- Return agent ids on stdout.
- Support both human-readable and JSON output formats.
- Add tests for argument parsing, status filtering, invalid status handling, empty agent directories, and output shape where practical.

# Validation
- `cargo fmt`
- `cargo test`
- `cargo run -- check`
