+++
name = "Fix ticket list human header alignment"
creation_date = 2026-07-07T10:57:20Z
status = "completed"
+++

# Goal

Fix `waap ticket list` human-readable table formatting so headers align with row values and use normal title case instead of all caps.

# Problem

Current output can visually misalign the `STATE` header because the renderer only computes a width for the ticket-id column. The state header is placed immediately after the short `STATUS` header, while row state values are placed after longer status values such as `completed`:

```text
TICKET ID                                                           STATUS  STATE
tt-add-ticket-functionality                                         completed
tt-validate-ticket-dependencies-in-waap-check                       completed  [unblocked]
```

The intended layout should align all columns and avoid all-caps headers, for example:

```text
Ticket ID                                                           Status     State
---------                                                           ------     -----
tt-add-ticket-functionality                                         completed  [unblocked]
```

# Implementation Notes

Relevant code is `src/ticket/list.rs`, especially `ticket_list_human_lines()`.

Likely changes:

- Change header constants from `TICKET ID`, `STATUS`, `STATE` to `Ticket ID`, `Status`, `State`.
- Compute a `status_width` from the max of the status header length and all rendered status values.
- Use that width for both the header row and every ticket row before the optional state column.
- Add a separator row below the header using dashes sized to each rendered column.
- Preserve empty-list behavior: no header or separator when there are no entries.
- Preserve JSON output exactly.

# Acceptance Criteria

- Human-readable `waap ticket list` output uses title-case headers: `Ticket ID`, `Status`, and `State` when the state column is present.
- Human-readable output includes a separator row below the header.
- The `State` header aligns with `[blocked]` / `[unblocked]` row markers even when status values are longer than `Status`.
- Rows without dependency state remain well-aligned when mixed with rows that have state.
- Empty human-readable ticket lists still produce no output.
- JSON output from `waap ticket list --output-format json` is unchanged.
- Tests cover header casing, separator row, status-width alignment, state-column alignment, and empty-list behavior.
- Developer validations pass:
  - `cargo clippy --all-targets -- -D warnings`
  - `cargo fmt --check`
  - `cargo build`
  - `cargo build --release`
  - `cargo test`
