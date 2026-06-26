+++
title = "Fix Agent Run Status Update"
creation_date = 2026-06-24T18:10:31Z
status = "pending"
+++

# Spec Reference
Lines 193-200 of /specs/spec.md

# Description
Fix `waap agent run` so it updates the agent metadata status when starting an agent, matching the spec.

Current behavior records `session_id` after creating the OpenCode session, but leaves `status = "ready"`. The spec says `waap agent run` starts the agent harness and updates the agent entry to `running`.

# Requirements
- When `waap agent run --agent-id <agent-id>` succeeds, update the agent frontmatter to `status = "running"`.
- Preserve existing behavior that records the OpenCode `session_id`.
- Ensure the human-readable and JSON run reports show the updated `running` status.
- Avoid marking an agent `running` if creating the OpenCode session or launching the detached OpenCode process fails.
- Add tests covering successful run metadata updates, including both `status` and `session_id`.
- Keep error behavior clear for invalid agent ids, missing agents, and missing OpenCode environment variables.

# Validation
- `cargo fmt`
- `cargo test`
- `cargo run -- check`
