+++
title = "Embed Agent Metadata In Agent Report"
creation_date = 2026-06-26T20:43:30Z
status = "completed"
+++

# Description
`AgentReport` (src/agent.rs) structurally duplicates `AgentMetadata`. The report is `{ agent_id, path, file_size }` plus copies of `creation_date`, `role`, `status`, and `session_id`, which `load_agent_report` copies field-by-field out of `AgentMetadata`. The JSON output already nests these under a `"metadata"` object, so the struct should embed the metadata rather than flattening copies of it.

This composes with `tt-derive-agent-run-report-from-metadata`; do that change as part of this work where the two overlap.

# Requirements
- Change `AgentReport` to embed the metadata, e.g. `{ agent_id: String, path: PathBuf, metadata: AgentMetadata, file_size: u64 }`.
- Update `load_agent_report` to construct the report without copying individual fields.
- Update `agent_report_json` and `print_agent_report_human` to read from `report.metadata`.
- Derive `Clone`/`PartialEq`/`Eq` on `AgentMetadata` as needed for the report's derives and tests.
- Preserve the existing human-readable and JSON output exactly, including the JSON nesting under `metadata` and the current set of fields shown (do not start emitting `system` in the report if it was not emitted before).
- Update all `AgentReport { .. }` constructions and assertions across the agent modules and their tests (run, get, update, agent.rs) to the embedded form.

# Validation
- `cargo fmt`
- `cargo test`
- `cargo run -- check`
