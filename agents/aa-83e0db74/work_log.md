# Work log — tt-implement-codex-app-server-json-rpc-client-srccodexrs

## Goal
Add `src/codex.rs`: minimal JSON-RPC client for `codex app-server --stdio`, plus
`CodexRunConfig` / `codex_run_config_from_env`. Protocol/client layer only.

## Protocol facts (verified against /home/cvoegele/code/github.com/openai/codex/codex-rs/app-server-protocol/src)
- Framing (`src/rpc.rs`): newline-delimited JSON, `jsonrpc` field omitted. Request =
  `{id, method, params?}`; notification = `{method, params?}` (no id); response =
  `{id, result}`; error = `{id, error{code,message,data?}}`.
- `initialize` (`protocol/v1.rs`): params `{clientInfo:{name,title?,version}, capabilities?}`
  (camelCase). Followed by `initialized` notification (no params). (README confirms.)
- `thread/start` (`protocol/v2/thread.rs`, camelCase): `{cwd, approvalPolicy, sandbox, model?}`.
  `approvalPolicy` is `AskForApproval` kebab-case → `"never"`. `sandbox` is `SandboxMode`
  kebab-case → `"danger-full-access"` for DangerFullAccess (`protocol/v2/shared.rs`).
  Response: `result.thread.id` (String, UUIDv7) (`protocol/v2/thread_data.rs`).
- `turn/start` (`protocol/v2/turn.rs`, camelCase): `{threadId, input:[UserInput], model?}`.
  UserInput is internally tagged `type`; text = `{"type":"text","text":...}` (textElements
  defaults). Response: `result.turn.id`.
- `turn/interrupt` (`protocol/v2/turn.rs`, camelCase): `{threadId, turnId}` → `{}`.
- Notifications: `item/agentMessage/delta` → `AgentMessageDeltaNotification` camelCase
  `{threadId, turnId, itemId, delta}`. `turn/completed` → `TurnCompletedNotification`
  camelCase `{threadId, turn}`.
- **TurnStatus** (`protocol/v2/turn.rs`) is `rename_all = "camelCase"`, so wire values are
  `"completed" | "interrupted" | "failed" | "inProgress"` — NOT PascalCase as the ticket
  guessed. I accept both spellings defensively but treat camelCase as canonical.

## Decision
Hand-define minimal serde structs/JSON builders with the exact renames above (citing source
files), rather than adding the codex-app-server-protocol crate as a dependency (it pulls a
large workspace and is not cleanly consumable). Client is generic over BufRead/Write/Write so
tests drive it with in-memory buffers.

## Steps
- [x] Read ticket, spec, claude.rs, opencode.rs, process.rs, main.rs.
- [x] Verified wire facts against codex source.
- [x] Implement src/codex.rs + register `mod codex;` in main.rs.
- [x] Tests; clippy/fmt/test/`cargo run -- check` all pass (165+ tests, 16 codex).
- [ ] Merge to main, mark ticket completed.

## Notes
- Module is not yet wired into `run_agent` (dependent ticket), so a scoped
  `#![allow(dead_code)]` keeps `-D warnings` green until the run flow lands.
- `child: Option<Child>` is held to keep the server alive; dropping `writer`
  EOFs stdin and tears it down. Dependent ticket adds SIGTERM/turn_interrupt.
