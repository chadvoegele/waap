+++
name = "Add global verbose logging mode"
creation_date = 2026-07-07T10:40:18Z
status = "pending"
depends_on = ["tt-root-resolution-and-waap-validation"]
+++

# Goal

Add global logging configuration for the `waap` CLI, including a verbose mode that helps diagnose root resolution.

# Background

After root resolution is made consistent by `tt-root-resolution-and-waap-validation`, users should be able to opt into diagnostic logging without changing command behavior. The first useful debug message should identify the resolved waap root after root resolution succeeds.

# Desired Behavior

- Add a global verbose flag, such as `--verbose` / `-v`, available to all `waap` commands.
- When verbose mode is enabled, initialize logging at debug level.
- Support `WAAP_LOG_LEVEL` as an environment variable for setting the log level when verbose mode is not provided.
- Prefer a popular Rust logging setup rather than ad hoc stderr printing. Suitable options include the `log` facade with `env_logger`, or `tracing` with `tracing-subscriber`; choose the smallest idiomatic fit for this CLI.
- After root resolution succeeds, emit a debug-level log containing the resolved waap root path.
- Keep normal command output clean, especially for `--output-format json`; logs should go to stderr or otherwise avoid contaminating stdout.

# Coordination

This should be blocked on `tt-root-resolution-and-waap-validation` because the logged root should reflect the finalized root-resolution behavior and should not reinforce the current special cases.

# Acceptance Criteria

- `waap --verbose <command>` enables debug logging for that invocation.
- `WAAP_LOG_LEVEL=debug waap <command>` enables debug logging without `--verbose`.
- `--verbose` takes precedence over `WAAP_LOG_LEVEL` if both are provided.
- A debug log records the resolved waap root after successful root resolution.
- JSON command output remains parseable when logging is enabled.
- Tests or documented manual validation cover verbose flag behavior, environment-variable log level behavior, and resolved-root debug logging.
- Developer validations pass:
  - `cargo clippy --all-targets -- -D warnings`
  - `cargo fmt --check`
  - `cargo build`
  - `cargo build --release`
  - `cargo test`
