+++
name = "Use dev shm for git-isolated root tests"
creation_date = 2026-07-07T12:12:15Z
status = "completed"
depends_on = ["tt-root-resolution-and-waap-validation"]
+++

# Goal

Change the root-resolution test helper that currently creates tempdirs under `/var/tmp` to prefer `/dev/shm`.

# Background

`src/root.rs` has a test helper:

```rust
fn tempdir_outside_any_git_repo() -> TempDir {
    tempfile::Builder::new().tempdir_in("/var/tmp").unwrap()
}
```

The helper exists so tests that assert "not inside a git repository" are not affected by stray `.git` directories above the default tempdir location. `/dev/shm` is a better scratch location when available because it is memory-backed and should also be outside the project git ancestry.

# Desired Behavior

- Update `tempdir_outside_any_git_repo()` in `src/root.rs` to use `/dev/shm` instead of `/var/tmp` when creating the tempdir.
- Keep the intent documented: the tempdir must be outside any ancestor `.git` directory so root-resolution tests are deterministic.
- Consider portability: if `/dev/shm` may not exist in some supported environments, use the smallest reasonable fallback or error message rather than making tests fail opaquely.
- Do not change production root-resolution behavior.

# Acceptance Criteria

- The root-resolution tests no longer hard-code `/var/tmp` as the primary tempdir base.
- Tests still verify the "not inside a git repository" cases deterministically.
- The helper comment accurately reflects the chosen tempdir base and why it is used.
- Developer validations pass:
  - `cargo clippy --all-targets -- -D warnings`
  - `cargo fmt --check`
  - `cargo build`
  - `cargo build --release`
  - `cargo test`
