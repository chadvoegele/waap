+++
name = "Prevent Agent Integration from Staling the Canonical Worktree"
creation_date = 2026-07-15T14:56:34Z
status = "pending"
+++

# Bug

`waap agent run` can leave the canonical repository worktree staged with old file versions after an agent merges from its managed worktree. This occurred while two OpenCode agents ran against the Leda repository, but the underlying failure does not require concurrency.

# Observed behavior

WAAP created isolated worktrees for agents `aa-5300df9e` and `aa-66b975c6`. Following the WAAP developer workflow, each agent rebased its branch and then integrated it by forcibly checking out `main` inside its agent worktree:

```sh
git switch --ignore-other-worktrees main
git merge --ff-only <agent-branch>
```

Both implementations and ticket-completion commits were present on `main`. After both `waap agent run` processes exited successfully, however, the canonical worktree reported staged reversions of their changes:

```text
D  .waap/agents/aa-5300df9e/work_log.md
D  .waap/agents/aa-66b975c6/work_log.md
M  .waap/tickets/tt-report-jina-reader-url-fetch-failures/ticket.md
M  .waap/tickets/tt-retry-transient-llm-cleanup-requests/ticket.md
M  src/leda/llm_scripter.py
M  src/leda/web_fetcher.py
M  tests/test_leda/test_llm_scripter.py
M  tests/test_leda/test_main.py
M  tests/test_leda/test_web_fetcher.py
```

The files and index in the canonical worktree still represented the pre-agent commit while its checked-out `main` reference had advanced. Reading ticket files from the canonical worktree consequently showed `status = "pending"`, even though `main` contained commits marking them completed.

The canonical worktree had no tracked changes before the agents started. Restoring only the listed paths from `HEAD` returned it to the correct clean state and exposed the completed tickets.

# Root cause

Git normally prevents one branch from being checked out in multiple worktrees. `--ignore-other-worktrees` bypasses that protection. Advancing `main` from an agent worktree updates the shared branch reference but does not refresh the index or files of the canonical worktree where `main` was already checked out.

Parallel runs make the state harder to reason about, but one agent can produce the same stale canonical checkout. The direct cause is allowing or instructing an agent to check out and advance `main` from its managed worktree.

# Expected behavior

- Agents make commits only on their isolated agent branches.
- Successful integration leaves the canonical worktree and index synchronized with `main`.
- Concurrent agents cannot race while updating `main` or WAAP metadata.
- `waap ticket list`, file reads, and `git status` immediately reflect merged ticket state after `waap agent run` exits.
- WAAP never requires `git switch --ignore-other-worktrees main`.

# Proposed fix

Move integration ownership from the developer agent to `waap agent run`:

1. Update the standard developer-agent instructions so agents commit and verify their agent branch, but never check out or merge `main`.
2. After the agent process exits successfully, have the runner acquire a repository-level integration lock.
3. Rebase the completed agent branch onto the latest `main` as needed.
4. Fast-forward `main` from the canonical repository worktree, or from one dedicated integration worktree that exclusively owns `main`, so its index and files are updated together.
5. Mark the ticket and agent completed through the same serialized integration path.
6. Remove the managed agent worktree only after integration and state validation succeed.
7. If the canonical worktree has conflicting tracked changes, stop with a clear recoverable error rather than moving `main` behind its index.

Do not use `--ignore-other-worktrees`. If multiple agent runs finish together, serialize only their integration phase; agent execution can remain parallel.

# Reproduction test

Add an integration test using a temporary Git repository:

1. Check out `main` in the canonical worktree.
2. Create two WAAP-managed agent branches and worktrees from the same base.
3. Commit distinct source, work-log, and ticket-status changes on each agent branch.
4. Complete both runs with overlapping execution and serialized integration.
5. Assert both commits are ancestors of `main`.
6. Assert the canonical worktree has the new file contents and completed ticket metadata.
7. Assert `git status --porcelain` is empty.
8. Assert no worktree other than the canonical worktree has `main` checked out.

Also cover one agent to ensure correctness does not depend on concurrency.

# Acceptance criteria

- Standard WAAP agent instructions no longer tell agents to merge into `main` from their worktrees.
- `waap agent run` owns and serializes branch integration.
- No production path invokes `git switch --ignore-other-worktrees main` or otherwise checks out `main` in multiple worktrees.
- Single-agent and concurrent-agent integration tests leave the canonical checkout clean and synchronized.
- Integration failure preserves the agent branch and worktree long enough for recovery and reports actionable details.
- Existing agent execution, status, session, and worktree lifecycle tests pass.
- Formatting, linting, unit tests, and `waap check` pass.
