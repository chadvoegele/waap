+++
name = "Generalize Git Worktree Helpers"
creation_date = 2026-07-08T02:41:37Z
status = "in-progress"
+++

Refactor Git worktree helpers so `git.rs` exposes generic Git operations rather than agent-specific helpers.

Scope:

- Replace `create_agent_worktree(waap_root, agent_id)` with a generic helper such as `create_worktree(repo_root, branch, relative_path)`.
- Replace `remove_agent_worktree(waap_root, agent_id)` with a generic helper such as `remove_worktree(repo_root, relative_path)`.
- Move the agent-specific `worktrees/<agent_id>` path convention out of `src/git.rs` and into the agent run layer, e.g. near `run_in_agent_worktree` in `src/agent/run.rs`.
- Keep the agent branch naming behavior unchanged: agent worktrees should still use the agent id as the branch name.
- Keep force removal behavior for cleanup so stale worktrees are removed even with uncommitted or untracked changes.
- Update imports and call sites in `src/agent/run.rs`.
- Update `src/git.rs` tests to cover the generic helpers.
- Keep agent run tests asserting that agents still launch inside `worktrees/<agent_id>` and clean up afterward.

Validation:

- Run `cargo fmt --check`.
- Run `cargo clippy --all-targets -- -D warnings`.
- Run `cargo build`.
- Run `cargo build --release`.
- Run `cargo test`.
