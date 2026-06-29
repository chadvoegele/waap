# AGENTS.md

## Developer Validations

Run all of the following from the repository root and ensure they pass **before making any commit**:

```sh
cargo clippy --all-targets -- -D warnings
cargo fmt --check
cargo test
```
