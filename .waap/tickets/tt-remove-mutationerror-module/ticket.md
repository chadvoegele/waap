+++
name = "Remove MutationError Module"
creation_date = 2026-07-08T02:23:22Z
status = "in-progress"
+++

Refactor mutation error handling to remove the dedicated mutation module.

Scope:

- Delete `src/mutation.rs`.
- Remove `mod mutation;` from `src/main.rs`.
- Move `Committed<T>` into `src/git.rs` near `commit_paths`.
- Replace `MutationResult<T>` return types with `std::io::Result<T>`.
- Replace `MutationError::Commit` mappings at each commit site with an `io::Error` that includes `failed to commit waap state change: {error}`.
- Simplify CLI mutation error handling in `src/app.rs` so command handlers print normal `io::Error` values with their command context.
- Update tests that currently pattern-match `MutationError::Operation` to assert directly on `io::Error`.

Expected commit failure output may include both command context and commit context, for example:

```text
failed to create agent: failed to commit waap state change: ...
```

Validation:

- Run `cargo fmt --check`.
- Run `cargo clippy --all-targets -- -D warnings`.
- Run `cargo build`.
- Run `cargo build --release`.
- Run `cargo test`.
