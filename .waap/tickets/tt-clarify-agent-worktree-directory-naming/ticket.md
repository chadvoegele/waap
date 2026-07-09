+++
name = "Clarify Agent Worktree Directory Naming"
creation_date = 2026-07-09T10:55:49Z
status = "abandoned"
+++

# Clarify Agent Worktree Directory Naming

Refactor the agent run code so `waap_root` is only used for the real waap project root and `worktree_dir` is used for agent execution directories.

## Background

`waap agent run` always executes an agent system inside an agent worktree. The current code initializes agent-system run configs with `waap_root`, then overwrites that same field with the prepared worktree path before launching the system. This works behaviorally, but the name is misleading and makes it harder to reason about which filesystem tree is being mutated or used for state.

There is also an opencode-specific helper, `run_opencode_in_agent_worktree`, even though opencode is never run outside a worktree. The opencode path should read like the claude and codex paths: `run_agent_opencode` should call the shared worktree lifecycle directly.

## Scope

Update opencode, claude, and codex agent run code consistently.

## Requirements

- Keep variables named `waap_root` only when they refer to the canonical waap project root / main repository state.
- Use `worktree_dir` when a value refers to the agent execution worktree.
- Rename agent-system run config fields that currently store the execution directory from `waap_root` to `worktree_dir` for:
  - `OpencodeRunConfig`
  - `ClaudeRunConfig`
  - `CodexRunConfig`
- Update all command/session construction to use `config.worktree_dir` where the agent process or remote session should operate inside the worktree.
- Inline/remove `run_opencode_in_agent_worktree`; `run_agent_opencode` should call `run_in_agent_worktree` directly, matching the claude and codex structure.
- Do the same structural cleanup for claude and codex if any extra helper or misleading naming remains after the `worktree_dir` rename.
- Preserve the existing worktree lifecycle behavior: prepare before worktree creation, run inside the worktree, cleanup after success, nonzero exit, or launch/session errors.
- Update tests to assert `waap_root` versus `worktree_dir` semantics clearly.
- Update comments that currently imply `config.waap_root` means the worktree.

## Validation

Run from the repository root:

```sh
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo build
cargo build --release
cargo test
```
