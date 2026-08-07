# Work log

- Read the ticket, Pi specification, backend implementation, and unit/process tests. Marked `tt-stabilize-pi-process-tests` in progress.
- Reproduced the flake under eight-way process contention after 250 sequential passes. Retained diagnostics showed request/child-close races returning `BrokenPipe`; the reported validation failure also executed freshly written scripts directly and returned `ExecutableFileBusy`.
- Reworked fake Pi fixtures to run non-executable scripts through a stable checked-in shell runner. Malformed JSON and EOF fixtures now consume `get_state` before responding or exiting. Timeout fixtures block on stdin instead of spawning `sleep` descendants.
- Added explicit child-reaping assertions to error paths and Pi process tests. The formerly flaky test passed 800 contended iterations; the five process tests pass with reaping checks.
- Repeated the complete Pi unit suite 100 times and the Pi process suite 50 times. Ran `cargo clippy --all-targets -- -D warnings`, `cargo fmt --check`, `cargo build`, `cargo build --release`, and `cargo test`; all passed.
- Committed `8e3559e`, rebased onto `origin/docs/pi-agent-system-spec`, fast-forwarded and pushed the existing PR #6 branch, and verified the remote head and Rust check passed.
