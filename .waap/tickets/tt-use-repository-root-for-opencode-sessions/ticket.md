+++
name = "Use Repository Root for OpenCode Sessions"
creation_date = 2026-07-11T12:36:44Z
status = "in-progress"
+++

# Problem

OpenCode sessions are currently created with the disposable agent worktree as their project directory. After an agent completes, waap removes that worktree, leaving OpenCode with a session whose directory no longer exists. OpenCode also scopes sessions by directory, so these worktree-scoped runs do not appear under the durable repository project in its UI.

# Required Behavior

- For `waap agent run --system opencode`, create the OpenCode session against the canonical waap repository root, not `worktrees/<agent-id>`.
- Launch the attached OpenCode CLI with the repository root as `--dir` and use that same root for OpenCode abort requests.
- Continue to create an isolated agent worktree at `worktrees/<agent-id>` and remove it after the run, as today.
- Change the OpenCode goal so it explicitly gives the absolute agent-worktree path and requires all implementation, Git, and validation work to happen there.
- Keep the agent instruction path resolvable from the OpenCode repository-root project.
- Limit the changed runtime-directory behavior to OpenCode; Claude and Codex must continue to run directly in the agent worktree.

# Tests and Documentation

- Add or update focused tests covering the OpenCode session directory, `--dir` argument, abort directory, and generated goal text containing the worktree path.
- Update `specs/spec.md` and `.agents/skills/waap/SKILL.md` to describe the repository-root OpenCode session and worktree-directed goal.

# Validation

```sh
cargo clippy --all-targets -- -D warnings
cargo fmt --check
cargo build
cargo build --release
cargo test
```
