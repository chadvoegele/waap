+++
title = "Add optional agent id to agent new"
creation_date = 2026-07-05T12:31:54Z
status = "completed"
+++

# Goal

Update `waap agent new` to accept an optional `--agent-id` argument.

# Requirements

- When `--agent-id` is provided, create the agent using that exact id.
- When `--agent-id` is not provided, keep the current behavior: generate an `aa-` id using the existing random hex mechanism.
- Validate a provided `--agent-id` using slug-style rules: lowercase ASCII letters, digits, hyphen, and underscore only; fewer than 64 characters; not empty.
- If the provided `--agent-id` already exists under `.waap/agents/`, return an error and do not overwrite it.
- Preserve existing `waap agent new` stdin behavior and output shape.

# Implementation Notes

- Add the CLI option to `AgentCommand::New`.
- Thread the optional id through `app.rs` into agent creation.
- Keep generated ids compatible with existing behavior.
- Add focused tests for parsing, valid custom id creation, invalid custom id rejection, duplicate rejection, and generated id fallback.

# Validation

Run the project validations required by `AGENTS.md` before committing:

```sh
cargo clippy --all-targets -- -D warnings
cargo fmt --check
cargo build
cargo build --release
cargo test
```
