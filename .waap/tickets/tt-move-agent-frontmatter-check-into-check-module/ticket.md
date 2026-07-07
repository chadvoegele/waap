+++
name = "Move agent frontmatter check into check module"
creation_date = 2026-07-07T10:53:08Z
status = "in-progress"
depends_on = ["tt-root-resolution-and-waap-validation"]
+++

# Goal

Move `check_agent_frontmatter` out of `src/agent.rs` and into `src/check.rs` so agent and ticket validation logic live together in the check module.

# Background

`check_agent_frontmatter(path: &Path, errors: &mut Vec<String>)` is currently defined in `src/agent.rs`, but usage inspection shows it is only imported and called from `src/check.rs`.

`src/check.rs` already handles ticket frontmatter validation directly inside `check_tickets()` using `parse_frontmatter` and `TicketMetadata::from_frontmatter`. Moving the agent equivalent into `check.rs` will mirror that structure and keep check-only validation helpers out of the agent domain module.

# Desired Behavior

- Remove `check_agent_frontmatter` from `src/agent.rs`.
- Define the equivalent helper in `src/check.rs`, likely as a private `fn` near `check_agents()`.
- Update imports so `src/check.rs` imports `AgentMetadata` as needed instead of importing `check_agent_frontmatter` from `crate::agent`.
- Preserve validation behavior and error messages exactly.
- Keep visibility minimal; the moved helper should not be `pub(crate)` unless there is a concrete external caller.

# Acceptance Criteria

- `check_agent_frontmatter` no longer exists in `src/agent.rs`.
- No non-check module imports or calls `check_agent_frontmatter`.
- Agent frontmatter validation in `waap check` behaves the same as before.
- The code structure in `src/check.rs` mirrors ticket frontmatter validation more closely.
- Developer validations pass:
  - `cargo clippy --all-targets -- -D warnings`
  - `cargo fmt --check`
  - `cargo build`
  - `cargo build --release`
  - `cargo test`
