+++
creation_date = 2026-06-23T12:21:29Z
role = "developer"
status = "completed"
session_id = "ses_10acb7258ffe6AKh0yS2i7dLxX"
+++

# Purpose
Implement code for `tt-implement-agent-new`.

# Instructions
Your role is to implement code for the functionality described in `/.waap/tickets/tt-implement-agent-new/ticket.md`. After implementing the code, write adequate unit tests, and end-to-end tests if appropriate. Verify with `cargo fmt`, `cargo test`, and `cargo run -- check`. Once the code is tested, merge it, resolving conflicts as necessary.

Be aware that many agents are editing the code simultaneously. Use git worktrees to isolate your changes before merging.

**git worktree example**
```
worktree_dir=worktrees/${agent_id}/tt-implement-agent-new
git worktree add -b ${agent_id}/tt-implement-agent-new ${worktree_dir}
pushd worktrees/${agent_id}/tt-implement-agent-new
# make changes, test, and merge
popd
git worktree remove ${worktree_dir}
```

Before you start, mark your ticket as in-progress with `waap ticket update --ticket-id tt-implement-agent-new --set-status in-progress` if that command is available. If it is not available yet, update the ticket frontmatter manually.

Include your agent id and the ticket id your worked on in your commit message.

When your code is merged and tested, complete your `/goal`.

If the ticket is already completed or abandoned, complete your `/goal`.

After completing your `/goal`,
1. mark your ticket as 'completed' with `waap ticket update --ticket-id tt-implement-agent-new --set-status completed` if available, or update the ticket frontmatter manually.
1. mark your status as 'completed' with `waap agent update --agent-id $agent_id --set-status completed` if available, or update this agent frontmatter manually.
