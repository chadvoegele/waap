+++
title = "Implement codex app-server JSON-RPC client (src/codex.rs)"
creation_date = 2026-06-29T19:06:18Z
status = "pending"
depends_on = ["tt-add-agentsystemcodex-variant-and-cli-wiring"]
+++

# Goal

Add `src/codex.rs`: a minimal JSON-RPC client that drives `codex app-server --stdio` over a spawned stdio child process, plus `CodexRunConfig` and `codex_run_config_from_env`. This is the protocol/client layer only — wiring it into `run_agent` is a separate dependent ticket.

# Spec References

- `/specs/codex-agent-system.md` §2 "JSON-RPC app-server client (`src/codex.rs`)", §"codex app-server protocol", and §7 "Config / env".

# Source Of Truth For The Protocol

The codex app-server protocol crate lives at `/home/cvoegele/code/github.com/openai/codex/codex-rs/app-server-protocol/` (crate `codex-app-server-protocol`, lib `codex_app_server_protocol`). Verified wire facts (read the source to confirm exact serde renames before encoding by hand):

- **Framing** (`src/rpc.rs`): newline-delimited JSON (JSONL) over stdin/stdout. The `"jsonrpc":"2.0"` field is **omitted** on the wire. Requests carry `id` (`String|i64`) + `method` + optional `params`. Responses carry `id` + `result`. Notifications carry `method` + optional `params` and **no `id`**. Errors carry `id` + `error{code,message,data}`.
- **`initialize`** (`src/protocol/v1.rs`): params `InitializeParams { clientInfo: { name, title?, version }, capabilities? }` (camelCase on wire). Response `InitializeResponse`. Followed by the **`initialized`** notification (no params).
- **`thread/start`** (`src/protocol/v2/thread.rs`): params `ThreadStartParams` (camelCase) with `cwd: String`, `model: Option<String>`, `approvalPolicy: AskForApproval`, `sandbox: SandboxMode`. For never-prompt + full access: `approvalPolicy = "never"` and the `DangerFullAccess` sandbox variant. CONFIRM the exact `SandboxMode`/`SandboxPolicy` wire encoding against the enum definition (the rename is `kebab-case` ⇒ likely `"danger-full-access"`; do not guess — read `shared.rs`). Response `ThreadStartResponse { thread: Thread, ... }`; the ThreadId is `response.thread.id` (`String`, UUIDv7). Server also emits `thread/started { thread }` and auto-subscribes the connection to the thread's events.
- **`turn/start`** (`src/protocol/v2/turn.rs`): params `TurnStartParams` (camelCase) with `threadId: String` (NOTE camelCase), `input: Vec<UserInput>` (list of text/image inputs — confirm the text-input shape), optional `model`. Response `TurnStartResponse { turn: Turn }`; the turn id is `response.turn.id` (`String`).
- **Streaming notifications** during a turn: `item/started`, `item/completed`, and `item/agentMessage/delta` (params `AgentMessageDeltaNotification { thread_id, turn_id, item_id, delta }` — concatenate `delta` per `item_id`).
- **`turn/completed`** (`turn.rs`): params `TurnCompletedNotification { thread_id, turn: Turn }`. `Turn.status` is `TurnStatus` with PascalCase wire values: `"Completed" | "Interrupted" | "Failed" | "InProgress"`. (Token usage arrives separately via `thread/tokenUsage/updated`; not required.)
- **`turn/interrupt`** (`turn.rs`): params `TurnInterruptParams { threadId, turnId }` (camelCase on wire). Response `{}`.

**Dependency decision (make this explicit in the implementation):** prefer adding `codex-app-server-protocol` as a path/git dependency in `Cargo.toml` and using its types directly to avoid hand-encoding mistakes. If that crate is not consumable as a dependency in this build, hand-define the minimal request/param/notification structs with `serde` using the exact field renames above, and add a comment citing the source files. Either way, do NOT invent field names.

# Current Implementation Context

- Mirror the shape of `src/claude.rs` and `src/opencode.rs` (config structs, `*_run_config_from_env`, `required_env` is in `opencode.rs`).
- `src/process.rs::run_forwarding` is for attached stdout/stderr forwarding of a child whose stdin is /dev/null; **codex does NOT use it** (it needs bidirectional stdio for JSON-RPC). This client spawns its own child with piped stdin AND stdout.
- Register the new module in `src/main.rs` (add `mod codex;`) alongside the other `mod` declarations.

# Required Behavior / Acceptance Criteria

1. `CodexRunConfig { model: Option<String>, repo_root: PathBuf }` and `codex_run_config_from_env(repo_root: &Path) -> io::Result<CodexRunConfig>` reading `CODEX_MODEL` via `env::var("CODEX_MODEL").ok().filter(|m| !m.is_empty())` and canonicalizing `repo_root`. It has no required vars (never fails for missing config), mirroring §7.
2. A function to spawn `codex app-server --stdio` as a child with piped stdin+stdout and `current_dir = config.repo_root` (the worktree). No prompt on argv.
3. A JSON-RPC client over the child's stdin/stdout that: writes newline-delimited JSON requests/notifications WITHOUT a `jsonrpc` field; reads newline-delimited inbound messages; correlates responses by `id`; and dispatches inbound notifications.
4. Typed client methods for the run flow: `initialize()` (sends `initialize`, waits for the response, then sends the `initialized` notification); `thread_start(cwd) -> io::Result<String>` returning `thread.id`, configured for never-prompt approvals (`approvalPolicy="never"`) and full sandbox access (DangerFullAccess), passing `model` when set; `turn_start(thread_id, prompt) -> io::Result<String>` returning `turn.id`; `turn_interrupt(thread_id, turn_id) -> io::Result<()>`; and `pump_until_turn_completed(thread_id, turn_id) -> io::Result<TurnStatus>` that forwards `item/agentMessage/delta` text to waap stdout and returns the final `TurnStatus` from `turn/completed`.
5. Define a `TurnStatus` type (or reuse the crate's) with at least `Completed`, `Failed`, `Interrupted`, `InProgress`, plus a helper like `is_success()` (`Completed` ⇒ true) for the completion logic the dependent ticket uses.
6. Errors (failed initialize/thread_start, malformed JSON, child stdin/stdout unavailable, EOF before `turn/completed`) return `io::Error` so callers can leave the agent `running`.

# Testing Expectations

Unit tests must not require a real `codex` binary. Test the pieces that are pure:
- Request framing: a request serializes to a single line of JSON with `id`/`method`/`params` and NO `jsonrpc` field; a notification has no `id`.
- `thread/start` params encode `approvalPolicy="never"`, the DangerFullAccess sandbox value, `cwd`, and include/omit `model` based on config (mirror `claude_run_command_omits_model_when_unset`).
- `turn/start` params use `threadId` (camelCase) and carry the prompt as input.
- `turn/interrupt` params use `threadId`/`turnId` (camelCase).
- Response/notification parsing: feed canned JSONL lines into the read/dispatch path and assert `thread_start` extracts `thread.id`, `turn_start` extracts `turn.id`, `pump_until_turn_completed` returns the right `TurnStatus` for a `turn/completed` with each status value, and `item/agentMessage/delta` deltas are forwarded/concatenated. Structure the reader over a generic `BufRead`/`Read` + `Write` (as `process.rs::forward` does) so tests can drive it with in-memory buffers without spawning a process.

# Developer Validations (must pass before merge)

- `cargo clippy --all-targets -- -D warnings`
- `cargo fmt --check`
- `cargo test`
- `cargo run -- check`
