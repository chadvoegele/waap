# AGENTS.md

Guidance for agents and developers working in the `waap` repository.

## Developer Validations

Run all of the following from the repository root and ensure they pass **before making any commit**:

```sh
cargo clippy --all-targets -- -D warnings
cargo fmt --check
cargo test
```

- `cargo clippy --all-targets -- -D warnings` — lint must be clean; warnings are treated as errors.
- `cargo fmt --check` — code must already be formatted (run `cargo fmt` to fix).
- `cargo test` — all unit and integration tests must pass.

Do not commit if any of these fail.
