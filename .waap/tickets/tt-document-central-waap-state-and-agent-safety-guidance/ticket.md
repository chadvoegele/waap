+++
name = "Document central waap state and agent safety guidance"
creation_date = 2026-08-02T23:45:59Z
status = "pending"
depends_on = ["tt-route-waap-commands-and-agent-runs-through-central-state"]
+++

Update user and agent documentation for central waap state after command activation.

## Scope

- Update `README.md`, `specs/spec.md`, `.agents/skills/waap/SKILL.md`, and bundled planner/developer role templates to match `specs/waap-state-worktree.md`.
- Use `~/.local/state/waap/data/...` consistently and reserve `.waap` for legacy application-checkout state.
- Explain state versus source worktrees, exact `--waap-root` semantics, setup-only `waap init`, explicit `waap repair`, remote behavior, relocation, and state-directory reports.
- Require CLI-based mutations. For emergency direct edits, document the expected dirty `waap check` failure, content correction, explicit commit on `waap`, and final clean check.
- Explain that state synchronization/push is manual and that remote-ahead warnings do not reconcile branches.
- Update examples and role prompts so agents never edit central state directly or create source worktrees from `waap`.
- Verify all cross-references and command examples against the implemented CLI.
