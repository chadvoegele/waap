+++
title = "Implement Agent Update"
creation_date = 2026-06-23T12:13:27Z
status = "completed"
+++

# Spec Reference
Lines 207-218 and 250 of /specs/spec.md

# Description
Implement the `waap agent update` CLI command so existing agent frontmatter can be updated without changing the agent id or markdown body content.

# Requirements
- Add `waap agent update --agent-id <agent-id> [--set-status <status>] [--set-session-id <session-id>]`.
- Validate the agent id format and that `.waap/agents/<agent-id>/agent.md` exists.
- Accept only valid agent statuses: `ready`, `running`, `completed`, and `aborted`.
- Require at least one of `--set-status` or `--set-session-id`.
- Preserve the existing agent `creation_date`, `role`, and markdown body while updating only requested metadata.
- Add or replace `session_id` when `--set-session-id` is provided.
- Report the updated agent path and metadata to stdout.
- Support both human-readable and JSON output formats.
- Add tests for argument parsing, missing agents, invalid statuses, missing update fields, frontmatter preservation, body preservation, and output shape where practical.

# Validation
- `cargo fmt`
- `cargo test`
- `cargo run -- check`
