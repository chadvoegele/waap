+++
creation_date = 2026-06-26T21:55:54Z
role = "developer"
status = "completed"
session_id = "c52199bc-1522-4dd0-871a-c1aa5276fb54"
system = "claude"
+++

# Purpose
Implement code for `tt-add-dependson-to-ticket-schema`.

# Instructions
Your role is to implement code for the functionality described in `/.waap/tickets/tt-add-dependson-to-ticket-schema/ticket.md`. After implementing the code, write adequate unit tests, and end-to-end tests if appropriate. Once the code is tested, merge it, resolving conflicts as necessary.

Be aware that many agents are editing the code simultaneously. Use git worktree's to isolate your changes before merging.

**git worktree example**
```
worktree_dir=worktrees/${agent_id}
git worktree add -b ${agent_id}/tt-add-dependson-to-ticket-schema ${worktree_dir}
pushd ${worktree_dir}
# make changes, test, and merge
popd
git worktree remove ${worktree_dir}
```

Before you start, mark your ticket as in-progress with `waap ticket update --ticket-id tt-add-dependson-to-ticket-schema --set-status in-progress`.

Include your agent id and the ticket id your worked on in your commit message. Use a fast-forward merge when possible to keep a linear history.

When your code is merged and tested, complete your `/goal`.

If the ticket is already completed or abandoned, complete your `/goal`.

After completing your `/goal`,
1. mark your ticket as 'completed' with `waap ticket update --ticket-id tt-add-dependson-to-ticket-schema --set-status completed`.
1. mark your status as 'completed' with `waap agent update --agent-id $agent_id --set-status completed`.
