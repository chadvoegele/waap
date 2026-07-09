+++
name = "Refactor Agent Worktree Lifecycle"
creation_date = 2026-07-09T12:07:18Z
status = "in-progress"
+++

# Refactor Agent Worktree Lifecycle

Refactor `waap agent run` so opencode, claude, and codex all use the same explicit worktree lifecycle shape without callback-heavy wrappers or misleading directory names.

## Background

Agent runs always execute inside an isolated git worktree at `worktrees/<agent-id>`. The current implementation hides that lifecycle behind `run_in_agent_worktree`, and opencode adds another callback-heavy wrapper, `run_opencode_in_agent_worktree`. This makes the production flow harder to read than the actual sequence of operations.

The current run configs also overload `waap_root`: configs are created with the canonical waap root, then the field is overwritten with the agent worktree directory. This works but makes it unclear whether a value refers to main repository state or the agent execution directory.

This supersedes `tt-clarify-agent-worktree-directory-naming`.

## Requirements

- Keep `waap_root` naming only for the canonical waap project root / main repository state.
- Use `worktree_dir` naming only for the agent execution worktree directory.
- Remove `run_opencode_in_agent_worktree`.
- Remove `run_in_agent_worktree` if practical, replacing it with an explicit, testable worktree lifecycle abstraction.
- Prefer a guard-style worktree owner, such as `AgentWorktree`, that:
  - creates the agent worktree after the running-state commit,
  - exposes `dir()` / `worktree_dir`,
  - removes the worktree on explicit cleanup,
  - attempts cleanup from `Drop` on early errors so launch/session failures do not leak worktrees,
  - still surfaces cleanup errors on the normal path.
- Make `run_agent_opencode`, `run_agent_claude`, and `run_agent_codex` read as direct sequential flows rather than callback orchestration.
- Avoid storing execution directories in agent-system config structs unless truly needed as configuration.
- Prefer passing `worktree_dir` explicitly to command/session builders:
  - opencode session creation and run command,
  - claude run command,
  - codex app-server spawn and `thread_start`.
- Keep agent-system configs limited to durable settings such as credentials, endpoint, and model. For stop operations, derive or pass the agent's `worktree_dir` explicitly rather than representing it as `waap_root` in a config.
- Preserve lifecycle ordering:
  - update/commit running state on `waap_root`,
  - create the agent worktree from that commit,
  - create or generate the system session as appropriate,
  - persist system/session metadata to `waap_root`,
  - run the agent system in `worktree_dir`,
  - cleanup the worktree,
  - finalize agent status on `waap_root`.
- Preserve opencode behavior where the session directory and run command directory are aligned to the worktree directory.
- Preserve codex behavior where the authentic thread id is persisted only after `thread/start` succeeds inside the worktree.
- Preserve claude behavior where the UUID session id is available before the worktree and included in the running-state commit.
- Update comments to distinguish main state operations from worktree execution.
- Define cleanup error precedence:
  - if the agent run fails and cleanup also fails, return the run error and retain cleanup failure context for diagnostics;
  - if the agent run succeeds and cleanup fails, return the cleanup error;
  - never silently discard cleanup failures.

## Testing

Update or replace existing callback-oriented tests with tests for the new structure:

- worktree guard creates and removes `worktrees/<agent-id>`,
- early errors trigger cleanup through the guard,
- cleanup errors are returned on the normal path,
- the worktree branch is cut from the running-state commit,
- opencode session/run directories use `worktree_dir`,
- claude command working directory uses `worktree_dir`,
- codex app-server and `thread_start` use `worktree_dir`,
- session ids are visible on `waap_root` while an opencode/codex run is active.
- a run failure combined with a cleanup failure preserves the run failure while retaining cleanup diagnostics.
- a successful run combined with a cleanup failure returns the cleanup error.

## Validation

Run from the repository root:

```sh
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo build
cargo build --release
cargo test
```
