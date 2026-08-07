+++
title = "Add Configurable Repository Root"
creation_date = 2026-06-25T10:55:35Z
status = "completed"
+++

# Spec Reference
Lines 124-128 of /specs/spec.md

# Description
Add a global CLI option that lets callers configure the repository root used by all waap operations.

Current code paths use `Path::new(".")`, for example `let errors = check_waap(Path::new("."));`. This makes commands depend on the current working directory. Waap should support operating on a specific repository path from any working directory.

# Requirements
- Add a global `--repo-root <path>` option.
- Default `--repo-root` to the current directory to preserve existing behavior.
- Apply `--repo-root` to every operation that reads or writes waap state, including `check`, `ticket new`, `ticket update`, `ticket get`, `ticket list`, `agent new`, `agent run`, `agent stop`, `agent update`, `agent get`, and `agent list`.
- Replace hard-coded uses of `Path::new(".")` in command dispatch with the configured repository root.
- Ensure OpenCode agent runs use the configured repository root for the OpenCode `--dir` value and session directory.
- Preserve existing human-readable and JSON output behavior except for paths naturally reflecting the configured root.
- Add tests covering at least one ticket operation, one agent operation, and `waap check` using a non-current repository root.
- Keep error messages clear when the configured path does not exist or cannot be accessed.

# Validation
- `cargo fmt`
- `cargo test`
- `cargo run -- check`
