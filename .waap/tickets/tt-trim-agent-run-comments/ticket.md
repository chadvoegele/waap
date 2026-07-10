+++
name = "Trim agent run comments"
creation_date = 2026-07-10T02:24:16Z
status = "pending"
+++

# Problem

`src/agent/run.rs` has accumulated too many explanatory comments. Many comments restate what the code already says, describe obvious control flow, or preserve historical rationale that now makes the file harder to scan.

# Desired Cleanup

Trim comments in `src/agent/run.rs` aggressively. Keep comments only when they explain something not obvious from the code itself, especially subtle lifecycle, Git/worktree, concurrency, signal-handling, or external-system behavior.

Examples of comments worth keeping or rewriting briefly:

- why worktree creation must happen after marking an agent running
- why session ids for some systems are persisted only after session creation inside the worktree
- why Codex completion is based on `TurnStatus` rather than process exit status
- non-obvious SIGTERM/interrupt behavior
- any invariant that would be easy to break during future refactors

Examples of comments to remove:

- comments that merely repeat function names or straightforward assignments
- long historical notes that no longer affect the current implementation
- acceptance-criteria comments inside tests when the assertion already communicates the behavior
- comments explaining ordinary read/write/commit sequencing unless there is a subtle reason

# Acceptance Criteria

- `src/agent/run.rs` has substantially fewer comments.
- Remaining comments are short and explain non-obvious behavior or invariants.
- No runtime behavior changes are made.
- Tests are not rewritten except to remove or shorten redundant comments.
- Run formatting and tests as appropriate for a comment-only Rust change:

```sh
cargo fmt --check
cargo test
```
