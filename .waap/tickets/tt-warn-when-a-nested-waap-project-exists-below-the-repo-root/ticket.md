+++
title = "Warn when a nested .waap project exists below the repo root"
creation_date = 2026-06-30T19:56:36Z
status = "in-progress"
+++

# Problem

`waap` resolves its state directory from `--repo-root` (default `.`, see `src/cli.rs`); it does not walk up or scan down. When a second waap project is nested in a subdirectory (e.g. `lab/.waap` and `lab/wiki/.waap`), commands run from the outer root operate only on the outer `.waap` and silently ignore the nested one.

This is a real footgun: an agent running in the nested `lab/wiki` project does not appear in `waap agent list --status running` run from `lab/`, making a correctly-`running` agent look missing. Nesting one waap project inside another is easy to do by accident and gives no signal.

# Desired Behavior

When a command resolves its repo root, detect any nested `.waap/` directory below that root and print a warning to **stderr** so the user knows another waap project exists that this invocation is not acting on.

Example:

```
warning: found nested waap project(s) below the repo root; this command only acts on <root>/.waap
  - wiki/.waap (use --repo-root wiki to target it)
```

Requirements:

- Scan subdirectories of `repo_root` for a `.waap` directory, excluding `repo_root/.waap` itself.
- Bound the walk: skip `.git`, `worktrees`, `target`, and `node_modules`; do not descend into a `.waap` directory once found (report its parent and stop descending that branch).
- Emit the warning to **stderr only**, never stdout, so `--output-format json` output stays clean and parseable.
- The warning is advisory: it never changes exit code or command behavior, and a failure while scanning must not abort the command.
- Show each nested project's path relative to `repo_root` and the `--repo-root <path>` value that would target it.

# Suggested Implementation

- Add a helper (e.g. `find_nested_waap_projects(repo_root) -> Vec<PathBuf>`) in a suitable module (e.g. `src/record.rs` or a small new module).
- Call it once near the top of `app::run` after `repo_root` is known, before dispatching the command, and print the warning if the result is non-empty.

# Acceptance Criteria

1. Running any command from an outer root that contains a nested `.waap/` (e.g. in `wiki/`) prints the nested-project warning to stderr.
2. No warning is printed when there are no nested `.waap/` directories below the root.
3. `repo_root/.waap` itself is never reported as nested.
4. `--output-format json` stdout is unchanged (warning goes to stderr); `worktrees/`, `.git/`, `target/`, and `node_modules/` are not scanned.
5. The scan does not change exit codes, and an error during scanning is non-fatal.
6. Tests cover: nested project detected, no nested project, and that the outer `.waap` is excluded.

# Validation

- `cargo clippy --all-targets -- -D warnings`
- `cargo fmt --check`
- `cargo test`
- `cargo run -- check`
