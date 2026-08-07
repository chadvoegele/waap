# Work Log: aa-4d77f687

Ticket: tt-add-ticket-status-to-ticket-list-output

## Goal
Surface each ticket's status in `waap ticket list` output (human + JSON).

## Changes
- `src/ticket/list.rs`:
  - Extracted human rendering into `ticket_list_human_lines` helper (testable, returns `Vec<String>`).
  - Human output now shows `{id}  {status}` (id-column aligned), preserving the
    `[blocked]`/`[unblocked]` marker for tickets with dependencies.
  - `ticket_list_json` now includes a `"status"` field per entry.
  - Updated `ticket_list_json_has_expected_shape` test for the new field.
  - Added `ticket_list_json_includes_status_field` and
    `ticket_list_human_lines_show_status` tests.

## Validation
- `cargo clippy --all-targets -- -D warnings` — pass
- `cargo fmt --check` — pass
- `cargo test` — 178 + extras pass
- `cargo run -- check` — OK: .waap is valid
