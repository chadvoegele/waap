+++
title = "Plan codex agent-system implementation"
creation_date = 2026-06-29T19:02:38Z
status = "completed"
+++

# Goal

Create the implementation tickets for adding `codex` as a third `waap agent run --system`, per the design in `/specs/codex-agent-system.md`.

# Instructions

Read `/specs/codex-agent-system.md` and create a dependency-ordered set of waap developer tickets that implement it. Study `/home/cvoegele/code/github.com/openai/codex` (`codex-rs/app-server/README.md`, `app-server-protocol/`) to pin exact JSON-RPC method/param/field names where the spec defers them to implementation.

Each ticket must be completable by a single developer agent and include: spec references (sections of `/specs/codex-agent-system.md`), the concrete waap files to touch, required behavior, acceptance criteria, and testing expectations. Use `depends_on` where ordering matters.

Suggested decomposition (adjust as you see fit), following the spec's sections:
1. `AgentSystem::Codex` enum + CLI wiring + tests (spec §1) — foundation, no deps.
2. `src/codex.rs` JSON-RPC app-server client (spec §2, §codex app-server protocol) — depends on (1).
3. `run_agent_codex` + dispatch + `finalize_codex_run` completion + worktree/config (spec §3, §6, §7) — depends on (2).
4. `waap agent stop` codex arm via signalling the run process (spec §5) — depends on (3).

Every developer ticket must require the AGENTS.md Developer Validations (`cargo clippy --all-targets -- -D warnings`, `cargo fmt --check`, `cargo test`) and `cargo run -- check` to pass before merge.
