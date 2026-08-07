+++
creation_date = 2026-06-20T12:59:21Z
role = "developer"
status = "completed"
session_id = "ses_11082695dffeb8110eLg6s6l9H"
+++

# Purpose
Implement code for `tt-add-ticket-functionality`.

# Instructions
Your role is to implement code for the functionality described in `/.waap/tickets/tt-add-ticket-functionality/ticket.md`. After implementing the code, write adequate unit tests, and end-to-end tests if appropriate. Verify with `cargo fmt`, `cargo test`, and `cargo run -- check`. Once the code is tested, merge it, resolving conflicts as necessary.

Be aware that many agents are editing the code simultaneously. Use git worktree's to isolate your changes before merging.

**git worktree example**
```
worktree_dir=worktrees/${agent_id}/tt-add-ticket-functionality
git worktree add -b ${agent_id}/tt-add-ticket-functionality ${worktree_dir}
pushd worktrees/${agent_id}/tt-add-ticket-functionality
# make changes, test, and merge
popd
git worktree remove ${worktree_dir}
```

Before you start, mark your ticket as in-progress with `waap ticket update --ticket-id tt-add-ticket-functionality --set-status in-progress`.

When your code is merged and tested, complete your `/goal`.

If the ticket is already completed or abandoned, complete your `/goal`.

After completing your `/goal`,
1. mark your ticket as 'completed' with `waap ticket update --ticket-id tt-add-ticket-functionality --set-status completed`.
1. mark your status as 'completed' with `waap agent update --agent-id $agent_id --set-status completed`.
