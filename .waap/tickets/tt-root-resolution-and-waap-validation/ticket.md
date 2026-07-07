+++
name = "Root Resolution And Waap Validation"
creation_date = 2026-07-07T02:37:30Z
status = "completed"
+++

# Goal

Make root resolution and command validation consistent when a git repository does not yet contain `.waap/`.

# Background

Current behavior special-cases `waap init` in `src/app.rs` because `resolve_waap_root()` requires `.waap/` to already exist. That makes `init` operate directly on `--waap-root` or `.` instead of sharing normal root resolution.

The desired behavior is to let root resolution identify the correct git-scoped project directory even before `.waap/` exists, while making uninitialized or invalid waap state fail clearly for commands that require it.

# Desired Behavior

Update root resolution so it follows these rules:

1. If `--waap-root` is provided and the directory exists, use that directory after verifying it is inside a git repository.
2. If a `.waap/` directory exists at or above the start directory, return the nearest directory containing `.waap/`, bounded by the current git root.
3. If no `.waap/` directory is found before reaching the git root, return the git root as the candidate waap root.
4. Do not search above the `.git` boundary. Linked worktree behavior must remain correct, where a `.git` file marks the worktree boundary.

Update validation so missing `.waap/` is not considered OK:

1. `waap check` should fail when `.waap/` is missing.
2. `check_waap()` should report a clear error such as `no waap project found; run 'waap init'` or equivalent.
3. Agent and ticket commands should validate initialized and valid waap state before operating.
4. Prefer running the same validation used by `waap check` before `waap agent ...` and `waap ticket ...` commands, so invalid `.waap/` state fails before command-specific logic mutates or reads state.

# Implementation Notes

Likely touch points:

- `src/root.rs`: change `resolve_waap_root()` fallback behavior and update tests that currently expect `NotFound` when no `.waap/` exists under the git root.
- `src/app.rs`: remove the `init` root-resolution special case if `resolve_waap_root()` can now return the git root before initialization.
- `src/check.rs`: make missing `.waap/` an error instead of returning an empty error list.
- `src/app.rs` or a small helper: run validation before agent/ticket command dispatch and print errors consistently.
- `src/record.rs`: keep or simplify `require_initialized_project()` depending on whether app-level validation fully covers those command paths.

# Acceptance Criteria

- `waap init` from a subdirectory of an uninitialized git repository initializes at the git root, not the subdirectory.
- `waap init --waap-root <dir>` initializes exactly at `<dir>` when `<dir>` exists and is inside a git repository.
- `waap check` in an uninitialized git repository exits non-zero and reports that `.waap/` is missing or that `waap init` is required.
- `waap agent ...` and `waap ticket ...` commands fail before operating when `.waap/` is missing.
- `waap agent ...` and `waap ticket ...` commands fail before operating when `check_waap()` reports invalid state.
- Existing linked-worktree root-resolution behavior remains covered by tests.
- Developer validations pass:
  - `cargo clippy --all-targets -- -D warnings`
  - `cargo fmt --check`
  - `cargo build`
  - `cargo build --release`
  - `cargo test`
