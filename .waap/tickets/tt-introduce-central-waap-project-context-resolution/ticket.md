+++
name = "Introduce central waap project context resolution"
creation_date = 2026-08-02T23:45:59Z
status = "in-progress"
+++

Implement the repository and state-resolution foundation in `specs/waap-state-worktree.md`.

## Scope

- Introduce a `ProjectContext` carrying the invocation worktree root, primary repository root, common Git directory, and state root.
- Without `--waap-root`, find the invocation worktree through its nearest `.git` entry, resolve `.git` files and `commondir`, require the supported `<primary-root>/.git` layout, canonicalize paths, and derive `~/.local/state/waap/data/<absolute-primary-path>`.
- Require absolute `HOME` and produce specific errors for unsupported bare and separate-Git-dir repositories.
- Resolve symlinks and ensure the primary checkout and all linked worktrees derive the same state root.
- Treat `--waap-root` as the exact state directory containing `agents` and `tickets`. Do not derive or inspect central or legacy state when it is supplied. Allow `waap init --waap-root` to target a path that does not exist yet.
- Resolve application source context separately so agent source operations use the invocation worktree rather than the state worktree.
- Add focused unit and integration tests for primary and linked worktrees, symlinks, explicit roots, invalid layouts, and path derivation.

## Self-hosting safety

This changes waap itself. Add the new context behind APIs without switching existing command dispatch from the repository's live `.waap` state. Do not run a newly built mutating waap command or `waap repair` against this repository during implementation. Use isolated temporary repositories for tests.
