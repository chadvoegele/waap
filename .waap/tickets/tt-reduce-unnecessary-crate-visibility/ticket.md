+++
name = "Reduce Unnecessary Crate Visibility"
creation_date = 2026-07-08T02:34:08Z
status = "pending"
+++

Tighten Rust visibility by making private-only helpers and modules private instead of `pub(crate)`.

Run this as the final cleanup after the pending structural refactors. The candidates below describe the current baseline: if an earlier ticket removed or renamed one, audit its replacement instead of restoring the old item solely to satisfy this list.

Scope:

- Change private-only helper functions from `pub(crate)` to private where they are only used in their defining module or child tests.
- Include at least these candidates:
  - `src/git.rs`: `agent_worktree_dir`
  - `src/check.rs`: `check_agents`, `check_tickets`, `read_dir`
  - `src/agent.rs`: `AgentMetadata::to_frontmatter_lines`, `AgentSystem::labels`
  - `src/ticket.rs`: `TicketMetadata::to_frontmatter_lines`
  - `src/ids.rs`: `slugify_name`
  - `src/record.rs`: `WaapRecordKind` helper methods used only inside `record.rs`
  - `src/agent/run.rs`: `print_run_agent_report`
  - `src/agent/stop.rs`: `agent_stop_json`, `stop_agents`
  - `src/agent/list.rs`: `agent_list_json`
  - `src/ticket/list.rs`: `ticket_list_json`
  - `src/agent/get.rs`: `agent_content_report_json`
  - `src/ticket/get.rs`: `ticket_get_report_json`
  - `src/agent/new.rs`: `create_agent_with_markdown`
  - `src/ticket/new.rs`: `create_ticket_with_markdown`
- Consider changing child module declarations in `src/agent.rs` and `src/ticket.rs` from `pub(crate) mod` to private `mod` where the intended crate-visible API is already re-exported by the parent module.
- Do not tighten struct fields or types in this pass unless the change is clearly local and non-cascading.
- Preserve tests that access helpers from same-file child test modules; child tests can still access private parent items through `super`.

Validation:

- Run `cargo fmt --check`.
- Run `cargo clippy --all-targets -- -D warnings`.
- Run `cargo build`.
- Run `cargo build --release`.
- Run `cargo test`.
