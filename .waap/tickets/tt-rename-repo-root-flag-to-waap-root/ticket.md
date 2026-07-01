+++
title = "Rename --repo-root flag to --waap-root"
creation_date = 2026-07-01T15:49:54Z
status = "completed"
depends_on = ["tt-resolve-repo-root-by-walking-up-to-nearest-waap-bounded-by-f778"]
+++

# Problem

The global CLI flag is named `--repo-root` (see `src/cli.rs`). After the root-resolution work (tt-resolve-repo-root-by-walking-up-to-nearest-waap-bounded-by-f778), "git root" is a distinct concept and the waap project root is not necessarily the git root, so `--repo-root` is a misnomer that conflates the two.

This ticket depends on that resolution ticket; rebase onto its changes before renaming.

# Desired Behavior

Rename the flag to `--waap-root`. It keeps the same semantics: it names the **directory that contains `.waap/`** (the project root), not the `.waap/` directory itself.

- Hard rename: **remove** `--repo-root` entirely. Do NOT keep it as a hidden alias.
- Update the flag's help text to describe it as the waap project root (the directory containing `.waap/`).
- Update all references to the flag name: the clap arg in `src/cli.rs`, tests that pass `--repo-root`, and any doc comments (e.g. `src/git.rs`) or docs (`.agents/skills/waap/SKILL.md`, role templates) that mention `--repo-root`. Grep the whole repo for `--repo-root` and `repo-root` to catch every reference.
- The internal Rust field/variable `repo_root` may be renamed to `waap_root` for consistency, but that is optional and cosmetic; the required change is the user-facing flag name. If renaming the field is churny, keep the internal name and only rename the flag.

# Acceptance Criteria

1. `waap --help` shows `--waap-root` and no longer shows `--repo-root`.
2. Passing `--repo-root` is rejected (unknown argument), confirming no hidden alias.
3. `--waap-root` behaves exactly as `--repo-root` did (same default `.`, same resolution/validation from the dependency ticket).
4. No remaining reference to `--repo-root` anywhere in the repo (`grep -rn -- --repo-root` returns nothing outside historical `.waap` records).
5. Help text describes `--waap-root` as the directory containing `.waap/`.

# Validation

- `cargo clippy --all-targets -- -D warnings`
- `cargo fmt --check`
- `cargo test`
- `cargo run -- check`
- `cargo run -- --help` (shows `--waap-root`, not `--repo-root`)
