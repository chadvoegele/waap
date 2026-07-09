+++
name = "Title Case Agent List Headers"
creation_date = 2026-07-09T02:35:23Z
status = "in-progress"
+++

# Summary

Update the human-readable agent list headers to match the title-case style used by ticket list.

# Problem

`src/agent/list.rs` currently defines agent list headers in all caps:

```rust
const AGENT_ID_HEADER: &str = "AGENT ID";
const STATUS_HEADER: &str = "STATUS";
```

Ticket list was updated to use title-case headers:

```rust
const TICKET_ID_HEADER: &str = "Ticket ID";
const STATUS_HEADER: &str = "Status";
const STATE_HEADER: &str = "State";
```

Agent list should use the same visual style for consistency.

# Desired Change

Change the agent list headers to title case:

```rust
const AGENT_ID_HEADER: &str = "Agent ID";
const STATUS_HEADER: &str = "Status";
```

Also use the same separator-row behavior as ticket list so the human-readable list formats remain consistent.

# Suggested Implementation

In `src/agent/list.rs`:

- Update `AGENT_ID_HEADER` from `"AGENT ID"` to `"Agent ID"`.
- Update `STATUS_HEADER` from `"STATUS"` to `"Status"`.
- Add a separator row like ticket list. Base it on the header label widths and pad it to the computed column widths.
- Update affected tests in `src/agent/list.rs`.

# Acceptance Criteria

- Human-readable `waap agent list` displays title-case headers.
- Column alignment remains correct when agent IDs are shorter or longer than the header.
- A separator row and header alignment match ticket list behavior.
- JSON output is unchanged.
- Existing tests pass; update or add focused tests as needed.

# Validation

Run the repository validations from `AGENTS.md` if implementing:

```sh
cargo clippy --all-targets -- -D warnings
cargo fmt --check
cargo build
cargo build --release
cargo test
```
