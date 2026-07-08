+++
name = "Simplify attached process output forwarding"
creation_date = 2026-07-08T10:48:42Z
status = "pending"
+++

## Context

`src/process.rs` currently runs attached agent commands with:

```rust
.stdout(Stdio::piped())
.stderr(Stdio::piped())
```

Then it manually copies child stdout/stderr back to this process's stdout/stderr using a foreground copy loop plus a stderr forwarding thread.

That is more complex than needed for the production behavior. `Stdio::piped()` only gives the parent process pipe handles; it does not automatically forward output. Since `run_forwarding()` only needs the child attached to the current terminal/stdout/stderr, `Stdio::inherit()` should be sufficient.

## Proposed Change

Simplify `src/process.rs`:

- Keep `.stdin(Stdio::null())`.
- Replace `.stdout(Stdio::piped())` with `.stdout(Stdio::inherit())`.
- Replace `.stderr(Stdio::piped())` with `.stderr(Stdio::inherit())`.
- Remove the manual pipe-draining/copy code and stderr forwarding thread.
- Keep `on_started()` running after successful `spawn()`.
- Return the child exit status via `child.wait()`.

## Test Updates

Update or remove tests that depend on custom `Vec<u8>` capture via the private `forward()` helper.

Keep coverage for:

- Child exit status is propagated.
- `on_started()` runs after spawn.
- stdin is connected to null so stdin-reading commands do not block.

## Validation

Run from the repository root:

```sh
cargo clippy --all-targets -- -D warnings
cargo fmt --check
cargo build
cargo build --release
cargo test
```

## Notes

Do not add a custom forwarding abstraction unless a real caller needs captured child output. Production attached output should rely on inherited stdout/stderr and normal shell redirection.
