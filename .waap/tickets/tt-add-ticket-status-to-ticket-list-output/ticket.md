+++
title = "Add ticket status to ticket list output"
creation_date = 2026-06-29T21:58:51Z
status = "completed"
+++

# Problem

`waap ticket list` output omits each ticket's status (pending / in-progress / completed / abandoned). Human output shows only the ticket id (and a blocked/unblocked marker); JSON shows `{ticket_id, blocked}`. Callers can't see status without opening each ticket or filtering one status at a time.

# Desired Behavior

Include the ticket status in `ticket list` output. The `TicketReport` already carries `status` (see `src/ticket/list.rs`), so it just needs to be surfaced.

- Human output: show the status alongside the id, e.g. `tt-foo  completed` (keep the existing blocked/unblocked marker). Choose a clear, aligned, stable format.
- JSON output: add a `"status"` field to each entry object, e.g. `{"ticket_id": "...", "status": "completed", "blocked": false}`.

# Implementation Notes

- `src/ticket/list.rs`: `print_ticket_list` (human) and `ticket_list_json` (JSON). Pull `entry.report.status`.
- Keep existing fields/markers for backward compatibility; only add status.

# Acceptance Criteria

1. `waap ticket list` human output shows each ticket's status.
2. `waap ticket list --output-format json` includes a `status` field per entry.
3. Existing blocked/unblocked behavior and the `--status`/`--unblocked` filters are unchanged.
4. Tests cover status presence in both human and JSON rendering.

# Validation

- `cargo clippy --all-targets -- -D warnings`
- `cargo fmt --check`
- `cargo test`
- `cargo run -- check`
