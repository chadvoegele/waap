+++
name = "Verify central waap state end to end"
creation_date = 2026-08-02T23:45:59Z
status = "pending"
depends_on = ["tt-document-central-waap-state-and-agent-safety-guidance"]
+++

Complete end-to-end verification of `specs/waap-state-worktree.md` and close implementation gaps.

## Scope

- Build isolated integration scenarios for every acceptance criterion in the specification: shared state across primary and linked worktrees, orphan/state-only history, remote adoption and warnings, source worktree selection, legacy migration, coexistence, exact overrides, dirty checks, conflicting branches, serialization, relocation, setup-only init, and reporting.
- Exercise fresh repositories, cloned repositories, repositories without remotes, unreachable remotes, moved repositories, and failure injection.
- Verify application branch HEADs and files remain unchanged by state mutations except the deliberate legacy-removal commit during migration.
- Run all developer validations and audit implementation and documentation against every normative statement in the spec.
- Fix uncovered defects, but do not weaken acceptance criteria to match implementation.

## Self-hosting safety

All destructive and migration scenarios must use disposable repositories. Do not migrate the live waap repository or replace the installed waap binary from an agent session.
