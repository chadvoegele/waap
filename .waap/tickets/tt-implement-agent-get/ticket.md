+++
title = "Implement Agent Get"
creation_date = 2026-06-24T17:59:46Z
status = "completed"
+++

# Spec Reference
Lines 218-224 of /specs/spec.md

# Description
Implement `waap agent get` so callers can retrieve an existing agent's metadata and markdown content.

# Requirements
- Add `waap agent get --agent-id <agent-id>`.
- Validate that `--agent-id` is a valid agent id.
- Load `.waap/agents/<agent-id>/agent.md`.
- Validate the agent frontmatter using the same rules as `waap check` and existing agent operations.
- Print agent metadata and content to stdout.
- Support both human-readable and JSON output formats.
- Return a non-zero exit code with a useful error message when the agent id is invalid or the agent does not exist.
- Include `session_id` in metadata when present.

# Validation
- `cargo fmt`
- `cargo test`
- `cargo run -- check`
