+++
name = "Move command commit and success printing out of app dispatch"
creation_date = 2026-07-07T10:46:43Z
status = "pending"
depends_on = ["tt-root-resolution-and-waap-validation"]
+++

# Goal

Refactor command execution so `src/app.rs` is only responsible for CLI dispatch plus consistent top-level error handling, while command modules own their state commits and success-report printing helpers own the output formatting details.

# Background

`src/app.rs` currently repeats a pattern for mutating commands:

```rust
match some_command(...) {
    Ok(report) => commit_and_print(
        waap_root,
        &[report.path.as_path()],
        "waap ...",
        |commit| print_some_report(&cli.output_format, &report, commit),
    ),
    Err(error) => { ... }
}
```

This spreads commit paths, commit messages, and report-printing details across the dispatcher. Examples include `init_project`, `create_agent`, `update_agent`, `stop_agents_with_systems`, `create_ticket`, and `update_ticket` call sites.

# Desired Behavior

- Move commit responsibility into the command functions/modules that mutate waap state.
  - For example, `init_project` should create the `.waap` skeleton and commit the relevant path itself, then return a report containing everything needed for output, including the commit id.
  - Apply the same pattern to mutating agent/ticket commands where `app.rs` currently calls `commit_and_print`.
- Move success-output details into the associated print report functions.
  - For example, `print_init_report` should print the full success output from the returned init report without `app.rs` passing a separate commit string.
  - Do the same for created/updated agent and ticket report printers, and agent stop output if it remains a mutating command.
- Keep `app.rs` focused on:
  - resolving the root,
  - dispatching to the appropriate command function,
  - calling the associated print function on success,
  - preserving the same user-facing error prefixes and exit-code behavior on failure.
- Remove the generic `commit_and_print` helper from `src/app.rs` if it no longer has callers.
- Preserve clean stdout behavior for JSON output; errors and commit failures should remain on stderr.

# Coordination

This should depend on `tt-root-resolution-and-waap-validation` because that ticket also changes `src/app.rs` command dispatch, especially the `init` special case and `Command::Init => unreachable!(...)` arm. Doing this after root resolution reduces merge conflicts and lets the refactor target the settled dispatch shape.

# Acceptance Criteria

- Mutating command modules perform their own `commit_paths` calls and return reports that include the commit hash when a commit is produced.
- `src/app.rs` no longer builds commit path slices or commit messages for normal mutating commands.
- `src/app.rs` no longer contains `commit_and_print` unless there is a remaining justified shared caller.
- Success report printer functions accept the final report shape and do not require `app.rs` to pass a separate commit string.
- Existing human-readable and JSON output for successful mutating commands remains compatible, including commit fields.
- Existing error prefixes and non-zero exit behavior remain compatible, including commit-failure errors.
- Developer validations pass:
  - `cargo clippy --all-targets -- -D warnings`
  - `cargo fmt --check`
  - `cargo build`
  - `cargo build --release`
  - `cargo test`
