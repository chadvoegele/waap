+++
creation_date = 2026-06-29T14:02:08Z
role = "developer"
status = "ready"
+++

# Purpose
Implement code for `tt-remove-role-from-agent-metadata-and-spec`.

# Instructions
Your role is to implement the functionality described in `/.waap/tickets/tt-remove-role-from-agent-metadata-and-spec/ticket.md`. After implementing the code, write adequate unit tests, and end-to-end tests if appropriate. Once the code is tested, merge it to the main branch, resolving conflicts as necessary.

Be aware that other agents may be editing the code simultaneously. Use a git worktree to isolate your changes before merging.

**git worktree example**
```
worktree_dir=worktrees/${agent_id}
git worktree add -b ${agent_id}/tt-remove-role-from-agent-metadata-and-spec ${worktree_dir}
pushd ${worktree_dir}
# make changes, test, and merge
popd
git worktree remove ${worktree_dir}
```

Before you start, mark your ticket as in-progress with `waap ticket update --ticket-id tt-remove-role-from-agent-metadata-and-spec --set-status in-progress`.

Run the validation checks in the ticket (e.g. `cargo fmt`, `cargo test`, `cargo run -- check`). Do not merge unless they pass.

Include your agent id and the ticket id you worked on in your commit message. Use a fast-forward merge when possible to keep a linear history.

If the ticket is already completed or abandoned, complete your `/goal` without code changes.

After your code is merged and tested,
1. mark your ticket as 'completed' with `waap ticket update --ticket-id tt-remove-role-from-agent-metadata-and-spec --set-status completed`.
1. mark your status as 'completed' with `waap agent update --agent-id ${agent_id} --set-status completed`.
