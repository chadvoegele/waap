+++
title = "Drive codex runs: run_agent_codex, dispatch, and turn-status completion"
creation_date = 2026-06-29T19:06:51Z
status = "pending"
depends_on = ["tt-implement-codex-app-server-json-rpc-client-srccodexrs"]
+++

# Goal

Wire the codex client into `waap agent run`: add `run_agent_codex` in `src/agent/run.rs`, dispatch `AgentSystem::Codex` to it, persist the authentic ThreadId as `session_id`, and derive completion from the `turn/completed` status (`finalize_codex_run`). This covers the happy-path run end-to-end (no graceful-stop signal handling yet — that is the next dependent ticket).

# Spec References

- `/specs/codex-agent-system.md` §3 "Driving a run (`run_agent_codex`)" (incl. the §3 "Completion" subsection), §4 "Session id", §6 "Worktree integration", §7 "Config / env".

# Current Implementation Context

- `src/agent/run.rs`:
  - `run_agent` match dispatches per `AgentSystem`. The earlier enum ticket left a `Codex` stub arm returning a not-implemented error — replace it with `run_agent_codex(repo_root, output_format, agent_id)`.
  - Model `run_agent_codex` structurally on `run_agent_opencode` (lines ~83-109): read the agent record, set `metadata.system = Some(AgentSystem::Codex)`, then call `run_in_agent_worktree(repo_root, agent_id, prepare = mark_running(...), run = |worktree| {...})`.
  - `run_in_agent_worktree` (lines ~64-81) is reused VERBATIM — note its `run` closure returns `io::Result<ExitStatus>`. codex's run produces a `TurnStatus`, not an `ExitStatus`, so do NOT force codex through that signature. Options (pick one and document it): (a) generalize `run_in_agent_worktree` over the run closure's return type `R` so the codex arm returns `TurnStatus` while opencode/claude keep `ExitStatus`; or (b) add a parallel `run_in_agent_worktree`-shaped helper for codex. Keep the existing opencode/claude paths and their tests unchanged in behavior either way.
  - `mark_running` (lines ~143-161) commits `running` to `main` BEFORE the worktree is cut, so `session_id` is unknown at that point. After `thread/start` returns the ThreadId inside the worktree, write it and commit (one extra commit per codex run) — implement `update_codex_session(repo_root, output_format, agent_id, thread_id)` mirroring `mark_running`'s write+`commit_paths` pattern (commit message e.g. `waap agent codex session <agent_id>`). `session_id` already exists on `AgentMetadata`; no schema change.
  - `finalize_agent_run`/`mark_completed` (lines ~163-207) key completion off `ExitStatus`. Add `finalize_codex_run(repo_root, output_format, agent_id, status: TurnStatus) -> io::Result<ExitCode>`: on `TurnStatus::Completed` mark the agent `completed` and commit (reuse `mark_completed`), returning `ExitCode::SUCCESS`; on `Failed`/`Interrupted`/`InProgress` leave the agent `running` and return a non-zero `ExitCode`. Consider refactoring `finalize_agent_run` to share a `success: bool` core with `finalize_codex_run` (the spec offers this as an alternative); avoid duplicating the mark/commit logic.
- `src/codex.rs` (prior ticket) provides `codex_run_config_from_env`, the spawn function, the client with `initialize`/`thread_start`/`turn_start`/`pump_until_turn_completed`, and `TurnStatus`.

# Required Behavior / Acceptance Criteria

1. `run_agent` dispatches `AgentSystem::Codex => run_agent_codex(...)`.
2. `run_agent_codex` flow (per §3): build config from env; read record; set `system = Codex`; `mark_running` (commits `running` on `main`); cut worktree; set `config.repo_root = worktree`; spawn `codex app-server --stdio` in the worktree; `initialize()`; `thread_start(worktree)` → ThreadId; `update_codex_session(...)` persists+commits the ThreadId as `session_id`; build the prompt `Complete when instructions in /.waap/agents/<agent_id>/agent.md are satisfied`; `turn_start` → turn id; `pump_until_turn_completed` → `TurnStatus`; the worktree is removed by `run_in_agent_worktree` even on error.
3. The prompt string matches the claude/opencode wording exactly (see `build_claude_run_command`).
4. `thread/start`'s `cwd` is the worktree path, so the model's tools operate inside `worktrees/<agent-id>` (§6). `CODEX_HOME` is inherited from the environment and is neither set nor relocated by waap.
5. Completion (§3 "Completion", §4): `TurnStatus::Completed` ⇒ agent marked `completed` on `main` + exit 0; non-`Completed` ⇒ left `running` + non-zero exit. `session_id` is the authentic ThreadId from `thread/start`.
6. A misconfigured environment surfaces as an `initialize`/`thread_start` error and leaves the agent `running` (§7) — i.e. the error propagates out of the run closure; the agent is not marked `completed`.

# Testing Expectations

- `finalize_codex_run` marks `completed` + commits on `TurnStatus::Completed` and leaves `running` + makes no commit on `Failed`/`Interrupted` — mirror the existing `finalize_agent_run_marks_completed_on_zero_exit` / `finalize_agent_run_leaves_running_on_nonzero_exit` tests (they show the git-fixture helpers `init_repo_with_commit`/`seed_agent_record`).
- `finalize_codex_run` never changes ticket status (mirror `finalize_agent_run_does_not_change_ticket_status`).
- `update_codex_session` writes `session_id`+`system = "codex"` into the record and commits it (assert frontmatter contents and the commit subject), mirroring `run_agent_claude_updates_status_and_session_id_in_frontmatter`.
- If `run_in_agent_worktree` is generalized over the return type, keep/extend its existing tests so opencode/claude (`ExitStatus`) behavior is unchanged and add a case exercising a non-`ExitStatus` return.

# Dependency Rationale

Depends on `tt-implement-codex-app-server-json-rpc-client-srccodexrs` because `run_agent_codex` calls the client (`spawn`, `initialize`, `thread_start`, `turn_start`, `pump_until_turn_completed`) and `finalize_codex_run` switches on its `TurnStatus`.

# Developer Validations (must pass before merge)

- `cargo clippy --all-targets -- -D warnings`
- `cargo fmt --check`
- `cargo test`
- `cargo run -- check`
