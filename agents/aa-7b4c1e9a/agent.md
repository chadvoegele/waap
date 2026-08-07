+++
creation_date = 2026-06-23T02:21:46Z
role = "developer"
status = "completed"
+++

# Purpose
Implement code for `tt-implement-agent-run`.

# Instructions
Your role is to implement code for the functionality described in `/.waap/tickets/tt-implement-agent-run/ticket.md`. After implementing the code, write adequate unit tests, and end-to-end tests if appropriate. Verify with `cargo fmt`, `cargo test`, and `cargo run -- check`. Once the code is tested, merge it, resolving conflicts as necessary.

Be aware that many agents are editing the code simultaneously. Use git worktree's to isolate your changes before merging.

**git worktree example**
```
worktree_dir=worktrees/${agent_id}_${ticket_id}
git worktree add -b ${agent_id}_${ticket_id} ${worktree_dir}
pushd worktrees/${agent_id}_${ticket_id}
# make changes, test, and merge
popd
git worktree remove ${worktree_dir}
```

When your code is merged and tested, complete your `/goal`.

If the ticket is already completed or abandoned, complete your `/goal`.
