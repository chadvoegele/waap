+++
title = "Move worktree management to agent run"
creation_date = 2026-06-29T00:00:00Z
status = "completed"
depends_on = ["tt-move-agent-run-to-attached"]
+++

# Problem

Agent worktree setup and cleanup are currently handled through agent instructions. That makes worktree lifecycle behavior dependent on whether the agent follows the prompt correctly, and it leaves cleanup inconsistent when an agent exits early or fails.

# Desired Behavior

`waap agent run` should own the agent worktree lifecycle. Before launching the selected system, it should create or prepare the worktree the agent should use. After the attached system process exits, it should clean up the worktree according to the intended waap lifecycle rules.

Agent instructions should no longer tell the agent to create or delete its own worktree. Agents should simply operate in the worktree prepared by `waap agent run`.

This ticket depends on `tt-move-agent-run-to-attached` because cleanup should happen after the system process exits, which requires `waap agent run` to remain attached to the system process.

# Acceptance Criteria

1. `waap agent run` creates or prepares the agent worktree before launching the selected system.
2. `waap agent run` launches both `opencode` and `claude` in the prepared worktree.
3. `waap agent run` deletes or cleans up the agent worktree after the selected system process exits, following the intended waap lifecycle rules.
4. Agent prompt/instruction generation no longer tells agents to create or delete their own worktree.
5. Worktree cleanup still runs when the selected system exits with a non-zero code.
6. Tests cover worktree creation, system launch directory, cleanup after success, cleanup after failure, and removal of worktree lifecycle instructions from generated agent instructions.
7. The spec.md and skill are updated to reflect the new worktree lifecycle.
