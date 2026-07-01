# AGENTS.md

## Developer Validations

Run all of the following from the repository root and ensure they pass **before making any commit**:

```sh
cargo clippy --all-targets -- -D warnings
cargo fmt --check
cargo build
cargo build --release
cargo test
```

`cargo test` spawns real `git` subprocesses (worktree, commit, init) in temporary
repositories, so run it outside any command sandbox; a sandbox blocks those
subprocesses and the `git::`, `agent::run::`, and `root::` tests fail with
`assertion failed: status.success()`.
