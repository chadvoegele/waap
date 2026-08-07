# Work Log

- Marked `tt-support-opencode-model-variant-via-providermodelvariant-in-30c9` in progress.
- Compared waap parsing with OpenCode's ACP parser and current model/variant catalog.
- Added model variant parsing, conditional `prompt_async` payload forwarding, validation, tests, and specification documentation while preserving slash-containing model IDs.
- Passed `cargo clippy --all-targets -- -D warnings`, `cargo fmt --check`, debug and release builds, `cargo test`, `cargo run -- check`, and `waap check`.
