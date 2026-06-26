+++
title = "Implement Agent New"
creation_date = 2026-06-23T12:18:35Z
status = "completed"
+++

# Spec Reference
Lines 182-192 of /specs/spec.md

# Description
Implement the `waap agent new` CLI command so new agent entries can be created in `.waap/agents/` from stdin.

# Requirements
- Add `waap agent new --role <role>`.
- Accept only valid agent roles: `developer` and `planner`.
- Generate an agent id as an 8 character random lowercase hex hash prefixed with `aa-`.
- Avoid id conflicts by generating another id when `.waap/agents/<agent-id>/` already exists.
- Create `.waap/agents/<agent-id>/agent.md` with TOML frontmatter containing `creation_date`, `role`, and `status = "ready"`.
- Append stdin content after the frontmatter.
- Report the created agent path, metadata, and file size to stdout.
- Support both human-readable and JSON output formats.
- Add tests for argument parsing, invalid roles, conflict handling, file creation, stdin content, generated metadata, file size reporting, and output shape where practical.

# Validation
- `cargo fmt`
- `cargo test`
- `cargo run -- check`
