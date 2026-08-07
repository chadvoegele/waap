# Work log: tt-add-agentsystemcodex-variant-and-cli-wiring

## Task
Add `AgentSystem::Codex` variant + CLI wiring (spec §1).

## Changes
- `src/agent.rs`: added `Codex` variant to `AgentSystem` (default stays `Opencode`);
  `as_str` returns `"codex"`. `parse`/`labels` iterate `value_variants()` so they pick
  it up automatically.
- `src/agent/run.rs`: added `AgentSystem::Codex` arm to `run_agent` returning an
  `Unsupported` io::Error stub, with a comment pointing at the dependent run ticket.
- Tests:
  - `src/agent.rs`: `agent_metadata_system_codex_passes` (frontmatter `system = "codex"`),
    `agent_system_codex_round_trips` (parse/as_str/labels).
  - `src/cli.rs`: replaced `agent_run_rejects_invalid_system_argument` (was using `codex`)
    with a positive `parses_agent_run_with_codex_system` test + invalid test using `cursor`.

## Validations
- cargo clippy --all-targets -- -D warnings
- cargo fmt --check
- cargo test
- cargo run -- check
