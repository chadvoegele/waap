# Work Log — aa-eb46b359

## Ticket
tt-sort-ticket-and-agent-lists-in-chronological-order

## Investigation
- `list_tickets` (src/ticket/list.rs) and `list_agents` (src/agent/list.rs) both
  build their result from `list_record_ids`, which returns ids sorted
  lexicographically — not chronologically.
- `TicketReport.creation_date` / `AgentReport.metadata.creation_date` are RFC
  3339 UTC strings, so string comparison is chronological.

## Changes
- `list_tickets`: after filtering, added
  `entries.sort_by(|a, b| a.report.creation_date.cmp(&b.report.creation_date));`
  (stable sort, applied post status/blocked filtering).
- `list_agents`: same pattern on `reports` using
  `metadata.creation_date`.
- Both human-readable and JSON output consume these sorted vectors directly,
  so no changes needed there.
- Added `ticket_list_orders_by_creation_date_not_id` and
  `agent_list_orders_by_creation_date_not_id` tests, with ids in an order
  that differs from creation-date order, to lock in the requirement.
- Added `write_ticket_with_creation_date` / `write_agent_with_creation_date`
  test helpers (existing `write_ticket`/`write_agent` now delegate to them
  with a fixed default date) so other tests didn't need touching.
- Ran `cargo fmt` to fix formatting of the new test.

## Verification
- `cargo clippy --all-targets -- -D warnings`: clean.
- `cargo fmt --check`: clean.
- `cargo build` / `cargo build --release`: clean.
- `cargo test` (outside sandbox, per AGENTS.md): 211 + 2 + 11 passed, 0 failed.
