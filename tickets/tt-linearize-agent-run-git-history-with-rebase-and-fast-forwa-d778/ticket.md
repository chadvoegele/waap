+++
title = "Linearize agent-run git history with rebase and fast-forward merge"
creation_date = 2026-06-29T15:03:53Z
status = "completed"
+++

# Problem

`waap agent run` produces a forked, non-linear git history. The run command commits the agent's `status = "running"` change to `main` (via `mark_running`, using the main `repo_root`), but it creates the agent's worktree branch from `main`'s HEAD *before* that commit. So `main` and the agent branch diverge immediately, and the agent's later merge back to `main` is non-fast-forward. With parallel agents, every concurrent run commits its own run-status to `main`, compounding the divergence.

See `src/agent/run.rs` (`run_agent_claude`, `run_agent_opencode`, `run_in_agent_worktree`, `mark_running`) and `src/git.rs` (`create_agent_worktree`).

# Desired Behavior

Agent-run history should be linear.

- Commit the `running` status to `main` **before** cutting the agent worktree, so the worktree branch descends from the run-status commit and carries it.
- The agent (in its instructions / role) rebases its branch onto the current `main` HEAD and performs a `--ff-only` merge before finishing. Rebasing keeps history linear even when other agents advanced `main` during the run.
- Keep committing `running` to `main` (not to the worktree branch) so `waap agent list --status running` works from the main worktree while the agent is running.

# Acceptance Criteria

1. After a single agent run, `git log --graph` shows a linear history with no merge bubble.
2. `waap agent list --status running` from the main worktree reports the agent as running during the run.
3. The worktree branch contains the `running` commit (it was cut after that commit).
4. With two agents run back-to-back (and concurrently, where feasible), `main` history remains linear after both merge.
5. Agent role/instruction docs describe the rebase + `--ff-only` merge step.
6. Tests cover worktree-branch base relative to the run-status commit and the linear-history outcome.

# Validation

- `cargo clippy --all-targets -- -D warnings`
- `cargo fmt --check`
- `cargo test`
- `cargo run -- check`
