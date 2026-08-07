+++
title = "Add status column and heading row to agent list output"
creation_date = 2026-07-01T21:35:59Z
status = "completed"
+++

# Add agent status to `waap agent list` human-readable output

## Motivation

The human-readable output of `waap agent list` prints only agent ids, so there is no way to see each agent's status at a glance:

```
aa-1031f97a
aa-39d3ee88
```

## Requirements

- Show each agent's status alongside its id in the human-readable output format (`OutputFormat::HumanReadable`) of `waap agent list`.
- Print an aligned heading row above the rows labeling the id and status columns (e.g. `AGENT ID`, `STATUS`), matching the style being added to `waap ticket list`.
- Align columns using a width computed from the longest agent id and the header width.
- Do not print a header when there are no entries.
- The JSON output format (`--output-format json`) currently emits an array of agent id strings. Extend it to include status per agent (e.g. `[{"agent_id": ..., "status": ...}]`) so JSON consumers can also see status. Update the JSON shape test accordingly.

## Implementation notes

- Relevant code: `src/agent/list.rs`, specifically `print_agent_list` and `agent_list_json`. Agent status is available at `report.metadata.status`.

## Acceptance criteria

- `waap agent list` prints an aligned heading row and one row per agent showing id and status.
- `waap agent list --output-format json` includes each agent's status.
- Existing tests are updated and new unit tests cover the status column, heading row, alignment, and the JSON shape.
- `cargo clippy --all-targets -- -D warnings`, `cargo fmt --check`, and `cargo test` all pass.
