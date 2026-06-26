+++
creation_date = 2026-06-26T22:04:02Z
role = "developer"
status = "completed"
session_id = "9865e69f-5527-4f87-877a-3a3a23819597"
system = "claude"
+++

# Purpose
Implement code for `tt-validate-ticket-dependencies-in-waap-check`.

# Instructions
Your role is to implement code for the functionality described in `/.waap/tickets/tt-validate-ticket-dependencies-in-waap-check/ticket.md`. After implementing the code, write adequate unit tests, and end-to-end tests if appropriate. Once the code is tested, merge it, resolving conflicts as necessary.

Be aware that many agents are editing the code simultaneously. Use git worktree's to isolate your changes before merging.

**git worktree example**
```
worktree_dir=worktrees/${agent_id}
git worktree add -b ${agent_id}/tt-validate-ticket-dependencies-in-waap-check ${worktree_dir}
pushd ${worktree_dir}
# make changes, test, and merge
popd
git worktree remove ${worktree_dir}
```

Before you start, mark your ticket as in-progress with `waap ticket update --ticket-id tt-validate-ticket-dependencies-in-waap-check --set-status in-progress`.

Include your agent id and the ticket id your worked on in your commit message. Use a fast-forward merge when possible to keep a linear history.

When your code is merged and tested, complete your `/goal`.

If the ticket is already completed or abandoned, complete your `/goal`.

After completing your `/goal`,
1. mark your ticket as 'completed' with `waap ticket update --ticket-id tt-validate-ticket-dependencies-in-waap-check --set-status completed`.
1. mark your status as 'completed' with `waap agent update --agent-id $agent_id --set-status completed`.
