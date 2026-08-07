+++
creation_date = 2026-06-26T22:04:02Z
role = "developer"
status = "completed"
session_id = "8da68306-f6e3-4050-af61-f44760486f32"
system = "claude"
+++

# Purpose
Implement code for `tt-waap-ticket-new-depends-on-flag`.

# Instructions
Your role is to implement code for the functionality described in `/.waap/tickets/tt-waap-ticket-new-depends-on-flag/ticket.md`. After implementing the code, write adequate unit tests, and end-to-end tests if appropriate. Once the code is tested, merge it, resolving conflicts as necessary.

Be aware that many agents are editing the code simultaneously. Use git worktree's to isolate your changes before merging.

**git worktree example**
```
worktree_dir=worktrees/${agent_id}
git worktree add -b ${agent_id}/tt-waap-ticket-new-depends-on-flag ${worktree_dir}
pushd ${worktree_dir}
# make changes, test, and merge
popd
git worktree remove ${worktree_dir}
```

Before you start, mark your ticket as in-progress with `waap ticket update --ticket-id tt-waap-ticket-new-depends-on-flag --set-status in-progress`.

Include your agent id and the ticket id your worked on in your commit message. Use a fast-forward merge when possible to keep a linear history.

When your code is merged and tested, complete your `/goal`.

If the ticket is already completed or abandoned, complete your `/goal`.

After completing your `/goal`,
1. mark your ticket as 'completed' with `waap ticket update --ticket-id tt-waap-ticket-new-depends-on-flag --set-status completed`.
1. mark your status as 'completed' with `waap agent update --agent-id $agent_id --set-status completed`.
