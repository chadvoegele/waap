+++
name = "Pi backend integration verification"
creation_date = 2026-08-07T17:19:55Z
status = "in-progress"
depends_on = ["tt-pi-rpc-agent-backend"]
+++

# Verify the Pi backend end to end

Review and complete the implementation of `specs/pi-agent-system.md` on PR branch `docs/pi-agent-system-spec` (PR #6) after the implementation tickets land.

## Scope

- Audit the implementation against every requirement and checklist item in the spec; fix gaps rather than only reporting them.
- Add process-level integration tests around the built `waap` CLI and a deterministic fake `pi` executable. Cover a successful run, authentic session persistence before prompt execution, Pi as the omitted-system default, failure, and stop/abort cleanup.
- Verify running records without `system` fail stop without mutation.
- Verify Pi and Codex interrupted owners converge on `aborted`, clean worktrees, and exit nonzero without transition errors.
- Update `README.md` and `.agents/skills/waap/SKILL.md` for Pi as the default system, shared model/reasoning options, required Pi installation/auth, and explicit backend selection.
- Keep provider-backed testing opt-in; do not require credentials in the default suite.

## Acceptance criteria

- All spec requirements have implementation or test coverage.
- `cargo clippy --all-targets -- -D warnings`, `cargo fmt --check`, `cargo build`, `cargo build --release`, and `cargo test` pass.
- Relevant process-level integration tests pass and do not call a model provider.
- `waap check` passes.
- Changes are committed, integrated into `docs/pi-agent-system-spec`, pushed to PR #6, and this ticket is completed.
