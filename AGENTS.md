# AGENTS.md

## Developer Validations

Run all of the following from the repository root:

```sh
cargo clippy --all-targets -- -D warnings
cargo fmt --check
cargo build
cargo build --release
cargo test
```

If you are an AI agent, run `cargo test` outside of any command sandbox.
