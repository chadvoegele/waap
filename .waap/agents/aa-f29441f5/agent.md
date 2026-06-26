+++
creation_date = 2026-06-24T10:50:21Z
role = "developer"
status = "completed"
session_id = "ses_106bdd7feffe5K5Dhwedo4IGv6"
+++

# Purpose
Implement code for `tt-refactor-waap-codebase`.

# Instructions
Your role is to implement code for the functionality described in `/.waap/tickets/tt-refactor-waap-codebase/ticket.md`. After implementing the code, write adequate unit tests, and end-to-end tests if appropriate. Verify with `cargo fmt`, `cargo test`, and `cargo run -- check`. Once the code is tested, merge it, resolving conflicts as necessary.

Be aware that many agents are editing the code simultaneously. Use git worktrees to isolate your changes before merging.

**git worktree example**
```
worktree_dir=worktrees/${agent_id}/tt-refactor-waap-codebase
git worktree add -b ${agent_id}/tt-refactor-waap-codebase ${worktree_dir}
pushd worktrees/${agent_id}/tt-refactor-waap-codebase
# make changes, test, and merge
popd
git worktree remove ${worktree_dir}
```

Before you start, mark your ticket as in-progress with `waap ticket update --ticket-id tt-refactor-waap-codebase --set-status in-progress` if that command is available. If it is not available yet, update the ticket frontmatter manually.

Include your agent id and the ticket id you worked on in your commit message.

When your code is merged and tested, complete your `/goal`.

If the ticket is already completed or abandoned, complete your `/goal`.

After completing your `/goal`,
1. mark your ticket as 'completed' with `waap ticket update --ticket-id tt-refactor-waap-codebase --set-status completed` if available, or update the ticket frontmatter manually.
1. mark your status as 'completed' with `waap agent update --agent-id $agent_id --set-status completed` if available, or update this agent frontmatter manually.
