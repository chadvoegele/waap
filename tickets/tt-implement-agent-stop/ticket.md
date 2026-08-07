+++
title = "Implement Agent Stop"
creation_date = 2026-06-24T18:00:01Z
status = "completed"
+++

# Spec Reference
Lines 201-206 of /specs/spec.md

# Description
Implement `waap agent stop` so callers can stop running agents and mark them as aborted.

# Requirements
- Add `waap agent stop` with optional `--agent-id <agent-id>`.
- When `--agent-id` is provided, stop only that agent.
- When `--agent-id` is omitted, stop all running agents.
- Mark stopped agents with `status = "aborted"` in their frontmatter.
- Validate agent ids and frontmatter using the same rules as existing agent operations.
- Decide and document how the command interacts with OpenCode sessions when `session_id` is present.
- If the current OpenCode API/client code can stop sessions, call it; otherwise clearly implement the metadata update and leave a note for future process/session termination support.
- Support both human-readable and JSON output formats.
- Return a non-zero exit code with a useful error message when the requested agent id is invalid or missing.
- Add tests for stopping one agent, stopping all running agents, filtering only running agents, and output shape.

# Validation
- `cargo fmt`
- `cargo test`
- `cargo run -- check`
