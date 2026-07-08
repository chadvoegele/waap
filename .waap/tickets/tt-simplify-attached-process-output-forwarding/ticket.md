+++
name = "Simplify attached process output forwarding"
creation_date = 2026-07-08T10:48:42Z
status = "in-progress"
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
- Replace the callback-oriented `run_forwarding(command, on_started)` API with a spawn-now/wait-later shape so lifecycle sequencing is explicit.
- Prefer configuring the existing `Command` directly. An adapter-specific spawn helper returning `std::process::Child` is acceptable when command fields remain private to the adapter.
- Preserve the current lifecycle: `run_in_agent_worktree` commits the running state before creating the worktree, then the run closure spawns the system and waits. Do not move `mark_running` after spawn; that would break the worktree branch-base invariant.

The intended call-site shape is:

```rust
let mut child = process
    .stdin(Stdio::null())
    .stdout(Stdio::inherit())
    .stderr(Stdio::inherit())
    .spawn()?;
let status = child.wait()?;
```

Prefer a synchronous process handle over a Rust `Future`; this code does not otherwise need async runtime machinery.

## Test Updates

Update or remove tests that depend on custom `Vec<u8>` capture via the private `forward()` helper.

Keep coverage for:

- Child exit status is propagated.
- The adapter returns a synchronous child handle that the run path waits on.
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
