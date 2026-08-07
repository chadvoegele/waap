# Work Log — aa-d80a4220

## Ticket
tt-add-status-column-and-heading-row-to-agent-list-output

## Investigation
- Read `src/agent/list.rs`: `print_agent_list` printed bare agent ids; `agent_list_json` emitted an array of id strings only.
- Looked at `src/ticket/list.rs` for the intended heading-row style (id column width computed via `max`, two-space separator). The matching ticket-list heading ticket (`tt-add-heading-row-to-ticket-list-human-readable-output`) is still `pending`/unimplemented, so `src/ticket/list.rs` itself has no header yet — used its column layout (`{id:width$}  {status}`) as the style reference per the ticket notes.

## Changes
- `src/agent/list.rs`:
  - Added `agent_list_human_lines` that computes `id_width` from the longest agent id and the `AGENT ID` header, returns `[]` for no entries, and otherwise returns a header line + one row per agent (`id  status`).
  - `print_agent_list` now prints `agent_list_human_lines` output for `HumanReadable` format.
  - `agent_list_json` now returns `[{"agent_id":..., "status":...}, ...]` instead of a plain id array.
  - Updated `agent_list_json_has_expected_shape` test for the new shape; added tests for heading/alignment/status on human lines and the empty-list case.

## Validation
- `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test` all pass locally.

## Status
Implementation complete; proceeding to rebase onto main and merge.
