+++
name = "Fix OpenCode agent runs using main checkout instead of worktree"
creation_date = 2026-07-08T13:21:17Z
status = "pending"
+++

# Problem

`waap agent run --system opencode` creates the OpenCode session before creating the agent worktree. The session therefore records the main checkout as its directory. WAAP later passes the agent worktree through `opencode run --dir`, but OpenCode routes session requests with `session.directory` taking precedence over the request directory. Agent tools consequently run in the main checkout.

A model-free reproduction confirmed the bug: create a session for a main checkout, then send that session a `pwd` request with a linked worktree as the request directory. OpenCode reports both the message cwd and command output as the main checkout.

The regression was introduced by commit `e8807b4`, which moved `create_opencode_session` ahead of `run_in_agent_worktree` so the session id could be included in the pre-worktree running-state commit. Before that commit, the session was created after `config.waap_root` was changed to the worktree.

Relevant code:

- `src/agent/run.rs`: `run_agent_opencode` and `run_in_agent_worktree`
- `src/agent/opencode.rs`: `create_opencode_session` and `build_opencode_run_command`
- OpenCode `WorkspaceRoutingMiddleware::planRequest`: session directory wins over the request's `directory`

# Required Behavior

- Create or retarget the OpenCode session so its persisted directory is the canonical agent worktree before sending the goal command.
- Do not rely on `opencode run --dir` to retarget an existing session.
- Keep the selected system and authentic OpenCode session id visible in the agent record on `main` while the run is active so `agent list` and `agent stop` continue to work.
- Preserve the intended Git history behavior: the agent branch includes the running-state commit and can be rebased and fast-forward merged without a merge bubble.
- Preserve worktree cleanup after session creation errors, launch errors, and non-zero OpenCode exits.
- Update `specs/spec.md` and the WAAP skill if the session/state commit ordering changes.

# Acceptance Criteria

1. The OpenCode session used by an agent run records `worktrees/<agent-id>` as its directory, not the main checkout.
2. OpenCode tool calls for that session execute in the prepared worktree. A `pwd`-equivalent assertion verifies the effective runtime cwd, not only the presence of `--dir` in command arguments.
3. The OpenCode run command and session creation target the same canonical worktree.
4. During the run, the main agent record contains `status = "running"`, `system = "opencode"`, and the session id; `waap agent stop` can abort that session.
5. The worktree branch contains the running-state commit, and existing linear-history behavior remains covered.
6. Worktree cleanup still occurs on success, non-zero exit, session creation failure, and process launch failure.
7. Regression coverage fails if the session is created against the main checkout while only the later run command points at the worktree.
8. All developer validations pass.
