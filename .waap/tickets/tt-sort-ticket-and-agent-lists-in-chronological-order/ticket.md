+++
title = "Sort ticket and agent lists in chronological order"
creation_date = 2026-07-01T21:39:36Z
status = "pending"
depends_on = ["tt-add-heading-row-to-ticket-list-human-readable-output", "tt-add-status-column-and-heading-row-to-agent-list-output"]
+++

# Sort `waap ticket list` and `waap agent list` chronologically

## Motivation

Both `waap ticket list` and `waap agent list` currently order rows by id (id strings are ~random), which makes it hard to see recent activity. They should be ordered by creation date so the newest entries appear at the bottom.

## Requirements

- Sort `waap ticket list` output by ticket `creation_date` ascending (oldest first, newest last).
- Sort `waap agent list` output by agent `creation_date` ascending (oldest first, newest last).
- Apply the ordering to both the human-readable and JSON output formats.
- Ordering applies after status/blocked filtering.
- `creation_date` values are RFC 3339 / ISO 8601 timestamps in UTC (e.g. `2026-07-01T21:35:53Z`), so lexicographic string comparison already yields chronological order; a stable sort on the timestamp string is sufficient.

## Implementation notes

- Relevant code: `list_tickets` in `src/ticket/list.rs` and `list_agents` in `src/agent/list.rs`. Both currently rely on `list_record_ids` returning ids sorted by id.
- `TicketReport.creation_date` and `AgentReport.metadata.creation_date` hold the timestamps.

## Acceptance criteria

- `waap ticket list` and `waap agent list` return entries ordered oldest-to-newest by creation date.
- Unit tests cover the chronological ordering for both lists (including entries whose id order differs from creation-date order).
- `cargo clippy --all-targets -- -D warnings`, `cargo fmt --check`, and `cargo test` all pass.
