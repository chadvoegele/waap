+++
title = "Add waap init command; require .waap for mutating commands"
creation_date = 2026-07-01T15:18:18Z
status = "pending"
+++

# Problem

waap currently has no explicit way to create a project. A `.waap/` directory is created implicitly the first time `ticket new` or `agent new` runs against a `--repo-root` (default `.`). This makes the project root ambiguous and makes it easy to create state in the wrong directory (e.g. a parent, or a fresh clone that should have been initialized elsewhere).

# Desired Behavior

Add an explicit `waap init` command that creates the `.waap/` skeleton, and make the mutating commands require an already-initialized project instead of auto-creating one.

## `waap init [--repo-root <path>]`

- Create the `.waap/` project skeleton at the resolved root (default: current directory).
- Error if a `.waap/` already exists there.
- Error if the target directory is not inside a git repository (waap commits its state with git, so a project outside git is invalid).
- On success, print the initialized root (human-readable) or a JSON object with the path, respecting `--output-format`.
- Commit the newly created `.waap/` skeleton (consistent with how other state changes are committed), or leave it uncommitted only if that is simpler and documented — prefer committing for consistency with `ticket new`/`agent new`.

## Require an initialized project for mutating commands

- `ticket new` and `agent new` must no longer implicitly create `.waap/`. If no initialized project is found at the resolved root, error with a message that points the user at `waap init` (e.g. `no waap project found; run 'waap init'`).
- Read-only and other commands keep today's behavior of treating a missing project as empty where that already applies, except where ticket 2 (root resolution) changes it.

# Notes / Coordination

- This ticket only adds `init` and the "must be initialized" guard for mutating commands. The *root resolution* rules (walking up to the nearest `.waap`, git-root bounding, `--repo-root` validation) are handled by a separate ticket that should build on this one. Keep the "not initialized" error message compatible with that follow-up.

# Acceptance Criteria

1. `waap init` creates `.waap/` at the resolved root and errors if one already exists.
2. `waap init` errors when the target is not inside a git repository.
3. `ticket new` / `agent new` no longer create `.waap/`; they error with a message pointing to `waap init` when the project is missing.
4. `--output-format json` returns clean, parseable output for `init` (path on success; error on stderr).
5. Tests cover: init in a fresh git repo, init when `.waap` already exists (error), init outside git (error), and `ticket new`/`agent new` erroring when uninitialized.

# Validation

- `cargo clippy --all-targets -- -D warnings`
- `cargo fmt --check`
- `cargo test`
- `cargo run -- check`
