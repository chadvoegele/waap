# Work Log

- Marked `tt-make-runner-completion-idempotent` in progress.
- Made runner-owned completed and failed persistence return immediately when the persisted status already matches.
- Added regression coverage for self-persisted completion and failure, preserved backend exit codes and runner errors, direct idempotence without duplicate commits, and conflicting terminal states.
- Verified `cargo clippy --all-targets -- -D warnings`, `cargo fmt --check`, `cargo build`, `cargo build --release`, and `cargo test` pass.
