+++
title = "Allow --output-format and --repo-root after the subcommand"
creation_date = 2026-06-29T15:47:09Z
status = "completed"
+++

# Problem

Global options must currently be passed *before* the subcommand. Putting `--output-format` (or `--repo-root`) after the subcommand fails:

```
$ waap ticket list --status pending --unblocked --output-format json
error: unexpected argument '--output-format' found
```

This is surprising — most CLIs accept global flags anywhere. The cause is that `output_format` and `repo_root` are defined on the top-level `Cli` parser (`src/cli.rs`), so clap only accepts them ahead of the subcommand.

# Desired Behavior

`--output-format` and `--repo-root` are accepted both before and after the subcommand (and after subcommand-specific flags). For example, all of these work and are equivalent:

```
waap --output-format json ticket list --status pending --unblocked
waap ticket list --status pending --unblocked --output-format json
waap ticket list --output-format json --status pending --unblocked
```

# Implementation Notes

- In `src/cli.rs`, mark the top-level `output_format` and `repo_root` args `global = true` (e.g. `#[arg(long, value_enum, default_value = "human-readable", global = true)]`). clap propagates `global` args to subcommands so they parse in either position.
- Confirm the existing access pattern (`cli.output_format`, `cli.repo_root`) still resolves correctly with `global = true`.
- Keep the existing defaults (`human-readable`, `.`).

# Acceptance Criteria

1. `waap ticket list --status pending --unblocked --output-format json` succeeds and prints JSON.
2. `--output-format` and `--repo-root` work both before the subcommand and after subcommand flags, for `ticket` and `agent` subcommands.
3. The pre-subcommand form continues to work (no regression).
4. Defaults are unchanged when the flags are omitted.
5. Tests cover the post-subcommand position for `--output-format` (and `--repo-root`) on at least one ticket and one agent subcommand.

# Validation

- `cargo clippy --all-targets -- -D warnings`
- `cargo fmt --check`
- `cargo test`
- `cargo run -- check`
