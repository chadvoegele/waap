+++
name = "Remove redundant initialized project guard"
creation_date = 2026-07-08T10:55:40Z
status = "pending"
+++

## Context

`src/app.rs` now runs `check_waap(waap_root)` before dispatching every `agent` and `ticket` command. That validates that `.waap/` exists and is valid before command-specific code runs.

`src/record.rs` still has a lower-level guard:

```rust
pub(crate) fn require_initialized_project(waap_root: &Path) -> io::Result<()> {
    if waap_root.join(".waap").is_dir() {
        Ok(())
    } else {
        Err(io::Error::new(
            io::ErrorKind::NotFound,
            "no waap project found; run 'waap init'",
        ))
    }
}
```

It is called by `create_agent_with_markdown()` and `create_ticket_with_markdown()`, but this duplicates the app-level validation for CLI execution.

## Proposed Change

Remove the redundant lower-level initialized-project guard:

- Delete `require_initialized_project()` from `src/record.rs`.
- Remove calls to it from `src/agent/new.rs` and `src/ticket/new.rs`.
- Remove now-unused imports.
- Update or remove tests that directly assert `create_agent_with_markdown()` / `create_ticket_with_markdown()` fail with the `waap init` message.
- Keep app-level validation in `src/app.rs` as the single source of truth for requiring a valid waap project before agent/ticket commands.

## Behavioral Notes

After this change, lower-level helper functions may assume that command dispatch has already validated `.waap`. Do not add another replacement guard unless there is a concrete caller that bypasses `app.rs` in production.

## Validation

Run from the repository root:

```sh
cargo clippy --all-targets -- -D warnings
cargo fmt --check
cargo build
cargo build --release
cargo test
```
