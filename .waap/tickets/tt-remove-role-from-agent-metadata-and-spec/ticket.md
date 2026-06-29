+++
title = "Remove role from agent metadata and spec"
creation_date = 2026-06-27T16:41:29Z
status = "completed"
+++

# Spec Reference

Lines 36-45, 55-63, 184-194, and 333-346 of /specs/spec.md

# Description

Remove agent role from waap's active agent model, CLI, specifications, and skill documentation. Agent purpose should be carried by the agent instructions/content, not by a separate required `role` frontmatter field or `waap agent new --role` CLI argument.

Existing agent records may still contain `role`; validation should tolerate `role` as an optional deprecated field so `waap check` continues to pass for existing `.waap/agents/*/agent.md` files.

# Requirements

- Remove `role` from the agent metadata schema documented in specs and generated agent frontmatter.
- Remove `role` from runtime/reporting code paths where it is part of the active `AgentMetadata` model, command output, JSON output, or newly-created records.
- Keep agent frontmatter validation compatible with existing records by accepting an optional deprecated `role` field when present.
- Remove the required `--role` parameter from `waap agent new` and update command output/tests accordingly.
- Update `specs/spec.md` so the agent schema example, `waap agent new` CLI reference, and lifecycle examples no longer mention `role` or `--role`.
- Update README and all waap skill documentation examples that still show `waap agent new --role ...`, including `.agents/skills/waap/SKILL.md` and `.agents/skills/waap-heat-equation-e2e-test/SKILL.md`.
- Preserve existing behavior for `creation_date`, `status`, `session_id`, and `system` metadata.

# Acceptance Criteria

1. New agents are created without a `role` frontmatter field.
2. Existing agent frontmatter containing `role` continues to pass `waap check` as a deprecated optional field.
3. `waap agent new` accepts stdin content without requiring or accepting `--role`.
4. Specs and documentation do not describe role as active agent metadata or as a `waap agent new` argument.
5. Skill docs and examples do not include `waap agent new --role ...`.
6. Tests are updated for agent creation, validation, CLI parsing, reports/output, and documentation-sensitive examples where applicable.

# Validation

- `cargo fmt`
- `cargo test`
- `cargo run -- check`
