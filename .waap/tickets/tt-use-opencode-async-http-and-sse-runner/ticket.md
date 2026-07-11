+++
name = "Use OpenCode async HTTP and SSE runner"
creation_date = 2026-07-11T21:06:04Z
status = "pending"
depends_on = ["tt-use-repository-root-for-opencode-sessions"]
+++

# Problem

`waap agent run --system opencode` creates a server session over HTTP but then shells out to `opencode run --attach`. The OpenCode CLI subscribes to SSE for progress while also holding `POST /session/{id}/message` open until the complete multi-turn agent run finishes. Reverse proxies therefore need long idle timeouts, and a timeout disconnects WAAP even though OpenCode continues processing. The subprocess also adds an unnecessary runtime dependency on a matching OpenCode CLI.

# Required Behavior

- Replace the attached OpenCode subprocess with direct authenticated HTTP and SSE handling in the OpenCode backend.
- Preserve `OPENCODE_SERVER_URL`, `OPENCODE_SERVER_USERNAME`, `OPENCODE_SERVER_PASSWORD`, and `OPENCODE_SERVER_MODEL` configuration.
- Create the OpenCode session against the canonical WAAP repository root with the existing denied-interaction permissions.
- Subscribe to `GET /event` before submitting the prompt so fast completion events cannot be missed.
- Submit the worktree-directed goal through `POST /session/{sessionID}/prompt_async` using the configured provider/model, the `build` agent, and text parts expected by the OpenCode API.
- Apply Basic authentication and the canonical repository-root `directory` query to session creation, event subscription, prompt submission, and abort requests.
- Parse SSE framing correctly, including comments, keepalives, blank-line event boundaries, and multiline `data:` fields.
- Ignore events for other sessions sharing the repository event stream.
- Forward matching completed text, tool, step, and error events as JSON lines compatible with the current `opencode run --format json` output.
- Treat matching `session.status` idle as successful completion and matching `session.error` as failure. Premature EOF, malformed matching events, and HTTP failures must fail the run rather than report success.
- Preserve `waap agent stop` by aborting the remote session and ensure an aborted monitor cannot overwrite the persisted `aborted` state.
- Remove OpenCode process spawning and the runtime dependency on the `opencode` executable. Keep shared worktree, session persistence, cleanup, and lifecycle orchestration unchanged.
- Update `specs/spec.md` and `.agents/skills/waap/SKILL.md` to describe direct async HTTP submission and SSE monitoring.

# Acceptance Criteria

- OpenCode runs complete through `prompt_async` plus SSE without spawning `opencode run --attach`.
- The event subscription is established before prompt submission.
- Only events for the created session affect output or terminal state.
- Idle, server error, abort, malformed SSE/JSON, HTTP rejection, and premature disconnect paths produce the correct `RunOutcome` and agent status.
- Tests verify Basic auth and repository-root directory scoping on create, event, prompt, and abort requests.
- Tests verify model parsing, prompt payload, SSE framing and keepalives, foreign-session filtering, JSON-lines output, completion, failure, cancellation, and disconnect behavior using a local HTTP fixture.
- Existing Claude and Codex backends remain unchanged.
- `cargo clippy --all-targets -- -D warnings`, `cargo fmt --check`, `cargo build`, `cargo build --release`, `cargo test`, `cargo run -- check`, and `waap check` pass.
