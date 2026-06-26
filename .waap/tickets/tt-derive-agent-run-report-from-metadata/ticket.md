+++
title = "Derive Agent Run Report From Metadata"
creation_date = 2026-06-26T20:43:19Z
status = "completed"
+++

# Description
`waap agent run` keeps the persisted agent metadata and the printed agent report in sync by hand, which is error-prone and reads the agent file twice.

In both `run_agent_opencode` and `run_agent_claude` (src/agent/run.rs):
- `load_agent_report` reads `agent.md` once to build the `report`.
- `read_agent_record` reads `agent.md` a second time to get the `metadata`.
- After mutating `metadata` (`session_id`, `system`, `status`) and writing it, the same `session_id`/`status`/`file_size` values are set a second time on `report`.

This duplicate, parallel maintenance is exactly the class of drift that caused the `status` bug fixed in `tt-fix-agent-run-status-update`. `update_agent` (src/agent/update.rs) already does this correctly: mutate `metadata`, `write_agent_record`, then `load_agent_report` to re-derive the report from the written file.

# Requirements
- In `run_agent_opencode` and `run_agent_claude`, stop building/mutating `report` in parallel with `metadata`.
- Remove the leading `load_agent_report` call so `agent.md` is read once, not twice.
- After `write_agent_record` succeeds, derive the report via `load_agent_report` (matching `update_agent`).
- Preserve existing behavior: the agent frontmatter ends with `status = "running"`, the recorded `session_id`, and the correct `system`; the run report (human-readable and JSON) reflects the running status and session id; status is not written if session creation or process launch fails.
- Keep the OpenCode active-session warning behavior unchanged.
- Update or add tests so the run path is covered without relying on parallel report maintenance.

# Validation
- `cargo fmt`
- `cargo test`
- `cargo run -- check`
