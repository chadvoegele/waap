+++
title = "Add heading row to ticket list human-readable output"
creation_date = 2026-07-01T21:35:53Z
status = "in-progress"
+++

# Add heading row to `waap ticket list` human-readable output

## Motivation

The human-readable output of `waap ticket list` prints ticket id and status columns with no header, so it is not obvious what each column means:

```
tt-add-published-nvfp4-accuracy-to-day-0-table                      abandoned
tt-audit-day-0-context-skill-and-reference-usage                    completed
```

## Requirements

- Print a heading row above the ticket rows in the human-readable output format (`OutputFormat::HumanReadable`) of `waap ticket list`.
- Column headers should label the id and status columns (e.g. `TICKET ID`, `STATUS`), and the blocked/unblocked state column when present.
- The header must align with the existing column widths. The id column width is currently computed from the longest ticket id; include the header width in that computation so headers and rows stay aligned.
- Do not print a header when there are no entries (empty list should stay empty).
- Leave the JSON output format (`--output-format json`) unchanged.

## Implementation notes

- Relevant code: `src/ticket/list.rs`, specifically `ticket_list_human_lines` and `print_ticket_list`.

## Acceptance criteria

- `waap ticket list` prints an aligned heading row followed by the ticket rows.
- Existing tests are updated and new unit tests cover the heading row and alignment. Empty-list behavior is covered.
- `cargo clippy --all-targets -- -D warnings`, `cargo fmt --check`, and `cargo test` all pass.
