+++
title = "Make agent state-commit writes idempotent (skip no-op status writes)"
creation_date = 2026-06-29T21:58:50Z
status = "in-progress"
+++

# Problem

`mark_completed`, `mark_running`, and `update_codex_session` in `src/agent/run.rs` unconditionally write the agent record and call `commit_paths`, even when the record on `main` is already in the target state. `commit_paths` was recently hardened to tolerate no-op commits, but these call sites still do wasted work (re-write the file, stage, attempt a commit) and, in the completion case, can produce a redundant/empty "waap agent completed" commit when the agent already advanced its own status.

# Desired Behavior

Guard each state transition so it only writes + commits when the record is not already in the target state — an `ensure_*`-style check. For example:

```
ensure_agent_completed(repo_root, agent_id):
    read record from repo_root (main)        # same record commit_paths would commit
    if record.status == "completed": return  # no write, no commit
    record.status = "completed"; write; commit_paths(...)
```

Apply the same shape to:
- `mark_completed` — skip if status already `completed`.
- `mark_running` — skip the write/commit if status already `running` (still print the report).
- `update_codex_session` — skip if `session_id` and `system` already match the target values.

Read the status/fields from the record on `repo_root` (main) immediately before deciding, so a concurrent merge that already set the value is observed (this is exactly the codex case where the agent self-marked `completed`).

Keep the human/JSON report output behavior (still report the agent state); only the write+commit is conditional.

# Acceptance Criteria

1. `mark_completed` performs no write and no commit when the agent is already `completed`; still completes successfully.
2. `mark_running`/`update_codex_session` likewise skip the write+commit when already in the target state.
3. When the record is NOT yet in the target state, behavior is unchanged: write + exactly one commit.
4. The `commit_paths` no-op tolerance remains (this guard is in addition, not a replacement).
5. Tests cover the already-in-target-state (no new commit) and needs-update (one commit) cases for at least `mark_completed`.

# Validation

- `cargo clippy --all-targets -- -D warnings`
- `cargo fmt --check`
- `cargo test`
- `cargo run -- check`
