+++
title = "Plan: add codex as an agent-run system"
creation_date = 2026-06-29T15:44:51Z
status = "pending"
+++

# Goal

Produce a written implementation plan (do NOT implement) for adding `codex` as a third value of `waap agent run --system`, alongside the existing `opencode` and `claude`. The plan is for human review before any implementation ticket is created.

# Deliverable

A markdown plan committed at `specs/codex-agent-system.md`. No changes to `src/` behavior in this ticket (adding only the plan doc is fine). After writing the plan, merge it to `main` and mark this ticket completed.

# Source To Study

- Codex source: `/home/cvoegele/code/github.com/openai/codex`
  - Non-interactive entrypoint: `codex exec` (see `codex-rs/exec/src/cli.rs`, `docs/exec.md`).
  - Relevant flags: positional `[PROMPT]` (or stdin), `--json` (alias `--experimental-json`, JSONL events to stdout), `--output-last-message <FILE>`, global `--model`, `--skip-git-repo-check`, `--ephemeral`, `--dangerously-bypass-approvals-and-sandbox`, and `codex exec resume <SESSION_ID> | --last`.
- waap wiring to mirror:
  - `src/agent.rs` — the `AgentSystem` enum (`Opencode`, `Claude`) with `as_str`/`parse`/`labels`.
  - `src/cli.rs` — `--system` arg (`value_enum`), and the parsing tests.
  - `src/claude.rs` — `build_claude_run_command`, `run_claude_attached` (stdout/stderr forwarding + `on_started` hook), `kill_claude_session` (`pkill -f <session_id>`), config-from-env (`CLAUDE_MODEL`).
  - `src/opencode.rs` — the equivalent for opencode, including session creation.
  - `src/agent/run.rs` — `run_agent_claude`, `run_in_agent_worktree` (worktree lifecycle), `mark_running` (commit `running` before cutting the worktree), `finalize_agent_run` (mark agent `completed` on exit 0).
  - `src/agent/stop.rs` — `stop_agents_with_systems` abort dispatch per system.

# Plan Must Cover

1. New `AgentSystem::Codex` variant and its `as_str`/`parse`/`labels`/CLI `--system codex` wiring (and which tests change).
2. How to build the `codex exec` command: prompt text (mirror claude's "Complete when instructions in /.waap/agents/<id>/agent.md are satisfied"), JSON/output flags, model env var, sandbox/approval bypass, and working directory = the prepared worktree.
3. Attached run: reuse the shared `run_forwarding`/`on_started` pattern so stdout/stderr forward and the agent is marked `running`, and exit code propagates (for `finalize_agent_run` auto-complete).
4. Session-id strategy — THE key open question. Codex generates its own session UUID (not pre-assignable like claude's `--session-id`). Propose how waap obtains it for the `session_id` metadata and for `waap agent stop`: parse the `--json` JSONL stream for the session-created event, use `--output-last-message`, or fall back to process-based kill. State the tradeoffs and pick one.
5. `waap agent stop` abort path for codex (process kill vs. session-based), consistent with the chosen session-id strategy.
6. Worktree integration (codex launched inside `worktrees/<agent-id>`; note `--skip-git-repo-check`/`CODEX_HOME` considerations).
7. Config/env: model selection (e.g. a `CODEX_MODEL` env var mirroring `CLAUDE_MODEL`), and `CODEX_HOME`/auth assumptions.
8. Test plan mirroring the claude/opencode unit tests (command construction, exit-code propagation, `--system codex` parsing, stop dispatch).
9. A short list of open questions / risks for human review.

# Acceptance Criteria

1. `specs/codex-agent-system.md` exists on `main` and covers items 1-9 above with concrete codex flags and concrete waap function/file references.
2. No behavioral change to `src/` (plan/doc only).
3. `cargo run -- check` passes; `cargo fmt --check` and `cargo test` still pass (unchanged code).
4. The plan explicitly resolves the session-id acquisition strategy with a recommendation.

# Validation

- `cargo fmt --check`
- `cargo test`
- `cargo run -- check`
