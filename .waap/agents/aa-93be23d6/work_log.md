# Work Log: aa-93be23d6

## Ticket
tt-add-heading-row-to-ticket-list-human-readable-output

## Investigation
- Read `src/ticket/list.rs`: `ticket_list_human_lines` builds rows with `id_width`
  computed only from ticket ids, no header row. `print_ticket_list` just prints
  whatever lines `ticket_list_human_lines` returns, so adding the header there
  is enough — no changes needed to `print_ticket_list`.
- Checked `src/agent/list.rs` for a similar precedent (a sibling ticket adds a
  status column/heading there) but it hadn't landed yet, so no shared helper
  to reuse.

## Changes
- `src/ticket/list.rs`:
  - Added `TICKET_ID_HEADER`/`STATUS_HEADER`/`STATE_HEADER` constants.
  - `ticket_list_human_lines` now returns `Vec::new()` immediately for an
    empty entry list (no header printed).
  - `id_width` now takes the header label length into account so the header
    and rows stay aligned.
  - Prepends a header row, including the `STATE` column only when at least
    one entry has `depends_on` set (mirrors how the state column is only
    printed per-row when `depends_on` is present).
- Added tests: header+state column widths, header without state column,
  header widening for short ids, and empty list produces no lines. Updated
  the existing `ticket_list_human_lines_show_status` test for the new header
  line.

## Validation
- `cargo clippy --all-targets -- -D warnings` — pass
- `cargo fmt --check` — pass (ran `cargo fmt` once to fix formatting)
- `cargo test` — pass (206 + 2 + 11 tests)

## Status
Ticket marked `in-progress` before edits. Will mark `completed` after merge.
