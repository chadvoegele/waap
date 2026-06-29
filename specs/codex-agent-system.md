# Plan: add `codex` as a third agent-run system (via `codex app-server --stdio`)

## Status

Planning only — no `src/` behavior is changed by the ticket that produces this
document. This plan adds `codex` as a third value of `waap agent run --system`,
alongside `opencode` and `claude`, driven through **`codex app-server --stdio`**
(a per-run JSON-RPC child process), not `codex exec`.

**Operator decisions already made (review):**
- Full sandbox/approval bypass (match the claude path), expressed via the
  app-server thread/turn permission settings (see §2).
- Surface codex's event stream to the operator (the app-server analogue of
  keeping claude/opencode machine-readable output).

## Why app-server over `codex exec`

`codex exec` was the first draft. It was rejected because the only ways to get
codex's real session id are to scrape it off stdout or to smuggle a waap token
onto the argv, and stop would have to fall back to a PID kill. The
**app-server** protocol removes both problems:

- `thread/start` returns the real `ThreadId` **synchronously** in its response —
  no scraping, no argv token.
- `turn/interrupt` is a graceful, programmatic stop (the analogue of opencode's
  HTTP abort).

The cost is speaking JSON-RPC to the child instead of reading plain stdout, and
deriving completion from a `turn/completed` event rather than a process exit
code. Crucially, in `--stdio` mode the app-server is **spawned per run as a
child process** — the same process lifecycle as claude/exec — so there is **no
standing server to manage or assume running** (that is only the experimental,
remote-management `codex app-server daemon`, which this plan does not use).

## Background: how the existing systems are wired

All under `src/`:

- `src/agent.rs` — `AgentSystem` enum (`Opencode`, `Claude`) with
  `as_str`/`parse`/`labels`, persisted to/from agent frontmatter (`system = "…"`).
- `src/cli.rs` — `AgentCommand::Run { agent_id, system }`; `system` is a clap
  `value_enum` defaulting to `opencode`.
- `src/claude.rs` — `build_claude_run_command`, `run_claude_attached`
  (stdout/stderr forwarding + `on_started` hook), `kill_claude_session`
  (`pkill -TERM -f <session_id>`), `claude_run_config_from_env` (`CLAUDE_MODEL`).
- `src/opencode.rs` — the opencode equivalent, including HTTP **session
  creation** (`create_opencode_session` returns a real id up front) and
  HTTP **abort** (`abort_opencode_session`). The codex app-server path is the
  closest analogue to opencode: a real id up front + a graceful abort.
- `src/agent/run.rs` — `run_agent` dispatch, `run_agent_claude`/
  `run_agent_opencode`, `run_in_agent_worktree` (worktree lifecycle),
  `mark_running` (commits `running` to `main` *before* the worktree is cut),
  `finalize_agent_run`/`mark_completed` (marks the agent `completed` on a zero
  exit; non-zero leaves it `running`).
- `src/agent/stop.rs` — `stop_agents_with_systems` dispatches the per-system
  abort: `Opencode => abort_opencode_session`, `Claude => kill_claude_session`.
- `src/process.rs` — `run_forwarding`, the shared inherit-stdio + `on_started`
  primitive claude/opencode build on. Codex does **not** reuse this (it needs to
  own the child's stdin/stdout for JSON-RPC; see §3).

## Findings from the codex source

Studied at `/home/cvoegele/code/github.com/openai/codex` (`codex-rs`,
`app-server/README.md`, `app-server-protocol/`):

- **Process:** `codex app-server` (alias `--stdio` / `--listen stdio://`,
  default) speaks **JSON-RPC 2.0** over stdin/stdout as newline-delimited JSON
  (the `"jsonrpc":"2.0"` header is omitted on the wire). One connection per
  process; the process exits when the connection closes.
- **Handshake:** send `initialize` (with client metadata/capabilities), then the
  `initialized` notification. Any other request before this is rejected.
- **Start a thread:** `thread/start` (params include `cwd`, permission/sandbox
  overrides, optional `model`, `ephemeral`). The **response returns the thread
  object including its id**, and the server also emits a `thread/started`
  notification and auto-subscribes the connection to that thread's turn/item
  events. `ThreadId` is the resume id (`thread/resume`).
- **Run a turn:** `turn/start` with `{ threadId, input, … }` returns the new
  turn object (with a turn id) and emits `turn/started`. Streaming notifications
  follow: `item/started`, `item/completed`, `item/agentMessage/delta`, tool
  progress, etc.
- **Finish:** the server sends **`turn/completed`** with the final turn state
  (status + token usage) when the model is done or the turn is interrupted.
- **Interrupt:** `turn/interrupt { thread_id, turn_id }` → `{}`; the turn ends
  with `status: "interrupted"`.
- **Permissions/approvals:** `thread/start`/`turn/start` accept sandbox/approval
  overrides. To run fully unattended (the approved "match claude" decision), set
  the approval policy to never-prompt and the sandbox to full access on the
  thread (and/or per turn). Exact field names to be pinned during
  implementation from `codex app-server generate-json-schema`
  (`InitializeParams`, `ThreadStartParams`, `TurnStartParams`).

## 1. `AgentSystem::Codex` variant and CLI wiring

In `src/agent.rs`: add `Codex` to `AgentSystem`; `as_str` ⇒ `"codex"`. `parse`
and `labels` need no change (they iterate `value_variants()`), so frontmatter
`system = "codex"` validates and `--system codex` parses automatically.

In `src/cli.rs`: no structural change — `--system` is a `value_enum` over
`AgentSystem`.

**Tests that change:**
- `src/cli.rs::agent_run_rejects_invalid_system_argument` currently asserts
  `--system codex` is invalid. **Replace** it with a positive
  `parses_agent_run_system_codex` test plus a new negative test on a still-
  invalid value (e.g. `--system gemini`).
- Add the codex analogue beside `parses_agent_run_system_argument`.
- `src/agent.rs`: assert `AgentSystem::parse("codex") == Some(Codex)` and
  `Codex.as_str() == "codex"`.

## 2. A JSON-RPC app-server client (`src/codex.rs`)

Add `src/codex.rs` mirroring `src/opencode.rs`'s role (a client to a session-
oriented server), but over a spawned stdio child rather than HTTP.

Command to spawn the server (no prompt on the argv — the prompt is sent as turn
input):

```
codex app-server --stdio
```

launched with `current_dir = <worktree>`. Config:

```rust
pub(crate) struct CodexRunConfig {
    pub(crate) model: Option<String>,   // from CODEX_MODEL (optional)
    pub(crate) repo_root: PathBuf,       // set to the worktree at run time
}
```

A minimal JSON-RPC client over the child's stdin/stdout:
- write framed requests/notifications (newline-delimited JSON, no `jsonrpc`
  header), correlate responses by `id`, and dispatch inbound notifications to a
  handler;
- typed just enough for the methods waap uses: `initialize`, `initialized`,
  `thread/start`, `turn/start`, `turn/interrupt`, and the inbound
  `turn/completed` + `item/agentMessage/delta` (for operator output).

Permission/sandbox: configure `thread/start` (and/or `turn/start`) for
never-prompt approvals + full sandbox access, per the approved "match claude"
decision. Pin exact field names from the generated JSON schema during
implementation.

## 3. Driving a run (`run_agent_codex` in `src/agent/run.rs`)

Codex does not reuse `run_forwarding` (it owns the child's stdio for JSON-RPC).
Add `run_agent_codex` modeled structurally on `run_agent_opencode`
(`src/agent/run.rs`):

```rust
fn run_agent_codex(repo_root, output_format, agent_id) -> io::Result<ExitCode> {
    let mut config = codex_run_config_from_env(repo_root)?;

    let (mut metadata, body) = read_agent_record(repo_root, agent_id)?;
    metadata.system = Some(AgentSystem::Codex);
    // session_id (ThreadId) is unknown until thread/start returns.

    let outcome = run_in_agent_worktree(
        repo_root, agent_id,
        // mark_running commits "running" before the worktree is cut; session_id
        // is filled in by a second commit once thread/start returns (see below).
        || mark_running(repo_root, output_format, agent_id, &mut metadata, &body),
        |worktree| {
            config.repo_root = worktree.to_path_buf();
            let mut client = spawn_codex_app_server(&config)?;   // child + JSON-RPC
            client.initialize()?;
            let thread_id = client.thread_start(worktree)?;       // REAL id, synchronous

            // Second state update: persist the authentic ThreadId as session_id, commit.
            update_codex_session(repo_root, output_format, agent_id, &thread_id)?;

            let prompt = format!(
                "Complete when instructions in /.waap/agents/{agent_id}/agent.md are satisfied"
            );
            let turn_id = client.turn_start(&thread_id, &prompt)?;
            // Pump notifications: forward agentMessage deltas to waap stdout for
            // operator visibility; return the final turn status.
            let status = client.pump_until_turn_completed(&thread_id, &turn_id)?;
            Ok(status)   // TurnStatus: Completed | Failed | Interrupted
        },
    )?;

    finalize_codex_run(repo_root, output_format, agent_id, outcome)
}
```

Dispatch: extend `run_agent` (`src/agent/run.rs:50`) with
`AgentSystem::Codex => run_agent_codex(...)`. The worktree lifecycle
(`run_in_agent_worktree`, `mark_running`) is reused verbatim.

**Completion mapping (the one model change).** claude/opencode derive completion
from the process exit code via `finalize_agent_run`. Codex derives it from the
`turn/completed` **status**: `Completed` ⇒ success (mark agent `completed`,
exit 0), `Failed`/`Interrupted` ⇒ leave `running` and return a non-zero
`ExitCode`. Implement a small `finalize_codex_run` that applies the same
mark/commit logic as `finalize_agent_run`/`mark_completed` but keyed on the turn
status instead of an `ExitStatus`. (Alternatively, refactor `finalize_agent_run`
to take a `success: bool`; either keeps the "zero ⇒ completed" contract.)

## 4. Session id — RESOLVED

`session_id` = codex's authentic **`ThreadId`**, taken directly from the
`thread/start` response. No stdout scraping, no argv token, no PID needed for
identity. This is the genuine, resume-capable id (usable later with
`thread/resume`/`thread/fork`).

Ordering note: `mark_running` commits `running` *before* the worktree is cut, so
`session_id` is not known yet. `thread/start` returns the `ThreadId` once the
server is up inside the worktree; a second `update_codex_session` commit writes
`session_id` then. One extra commit per codex run — no schema change
(`session_id` already exists on `AgentMetadata`).

## 5. `waap agent stop` for codex — signal the run process (DECIDED)

Graceful `turn/interrupt` requires the **live JSON-RPC connection**, which is
held only by the running `waap agent run` process (R). A separate
`waap agent stop` invocation has no channel to the per-run stdio child, so it
cannot call `turn/interrupt` directly. The chosen design is to **signal R and
let R interrupt gracefully** — this is the only stop mechanism (no PID-kill /
process-group fallback path).

- `waap agent stop` sends `SIGTERM` to R for this agent, matched by R's unique
  argv `agent run --agent-id <agent-id>`:
  `pkill -TERM -f "agent run --agent-id <agent-id>"`. This pattern matches R,
  **not** the `codex app-server --stdio` child (which lacks the agent id), and
  is independent of whether R was started in the foreground or backgrounded
  (`nohup`/`setsid`).
- `run_agent_codex` installs a `SIGTERM` handler that calls
  `turn/interrupt(thread_id, turn_id)` over its live connection, closes the
  connection, and returns through `run_in_agent_worktree` so the worktree is
  cleaned up. The interrupted turn yields a non-`Completed` status, so
  `finalize_codex_run` leaves the agent `running` (it never marks `completed`),
  and `waap agent stop`'s own `aborted` write on the record is therefore stable.
- In `src/agent/stop.rs::stop_agents_with_systems`, add the `AgentSystem::Codex`
  arm. It needs the **agent id** (available in `stop_agent_if_running`), not the
  `session_id`, so the abort closure signature changes to pass `agent_id` (or
  both). This is the one place codex diverges from the claude/opencode
  `abort(system, session_id)` shape.

Inherent safety property (not a separate fallback code path): because a stdio
server exits when its stdin EOFs, if R ever dies without running its handler
(e.g. `SIGKILL`), the child app-server is still torn down automatically. waap
implements only the signal-R path above.

## 6. Worktree integration

`run_in_agent_worktree` already cuts `worktrees/<agent-id>` from the `running`
commit, runs inside it, and removes it afterward (even on error). Codex reuses
this unchanged: spawn `codex app-server --stdio` with `current_dir = worktree`
and pass that `cwd` to `thread/start` so the model's tools operate in the
worktree. `CODEX_HOME` (auth/config/sessions, default `~/.codex`) is inherited
from the environment and is **not** the worktree; waap neither sets nor
relocates it. Session rollout files persist under `$CODEX_HOME` outside the
worktree (not waap's concern); do not pass `ephemeral` so `thread/resume` stays
possible.

## 7. Config / env

- **Model:** add `CODEX_MODEL`, mirroring `CLAUDE_MODEL`.
  `codex_run_config_from_env` reads it with
  `env::var("CODEX_MODEL").ok().filter(|m| !m.is_empty())`; when set, pass it as
  the `model` field on `thread/start`/`turn/start`, else use codex's default.
- **`CODEX_HOME` / auth:** assumed pre-configured (API key or prior
  `codex login`), exactly as claude assumes its auth is present. Document as an
  operator precondition for `--system codex`. `codex_run_config_from_env` has no
  required vars (`CODEX_MODEL` optional), so it never fails for missing config;
  a misconfigured environment surfaces as an `initialize`/`thread/start` error.

## 8. Test plan

In `src/codex.rs` (unit, no real codex needed — exercise the framing/protocol
logic against an in-memory or scripted stdio peer):
- `codex_app_server_spawn_command_matches_spec` — assert program/args
  (`codex app-server --stdio`) and `current_dir`.
- JSON-RPC framing round-trip: request id correlation; `initialize` →
  `initialized` ordering; `thread/start` response parsing yields the `ThreadId`;
  `turn/start` returns a turn id; `turn/completed` status parsing
  (Completed/Failed/Interrupted); `turn/interrupt` request shape.
- `pump_until_turn_completed` maps each terminal status to the right outcome.

In `src/cli.rs`: replace `agent_run_rejects_invalid_system_argument` with
`parses_agent_run_system_codex` + a new negative test.

In `src/agent.rs`: round-trip `system = "codex"`.

In `src/agent/run.rs`: a `finalize_codex_run` test (Completed ⇒ agent
`completed` + zero exit; Failed/Interrupted ⇒ `running` + non-zero), mirroring
the existing exit-code finalize tests.

In `src/agent/stop.rs`: a codex stop test asserting the `Codex` arm fires for a
running `system = "codex"` agent and the record becomes `aborted` — modeled on
`agent_stop_kills_claude_process_instead_of_opencode_abort`, adapted to the
agent-id-based signal of design (A).

## 9. Open questions / risks for human review

1. **Stop design, §5 — DECIDED.** Signal the run process via
   `pkill -TERM -f "agent run --agent-id <id>"`; R traps `SIGTERM` and calls
   `turn/interrupt`. No PID-kill / process-group fallback. This adds a SIGTERM
   handler in the run path and changes the codex abort arm to take `agent_id`
   instead of `session_id`.
2. **Completion-model change, §3.** Codex success comes from `turn/completed`
   status, not a process exit code, so it does not flow through the existing
   `finalize_agent_run` unchanged. Confirm a `finalize_codex_run` (or a
   `success: bool` refactor of `finalize_agent_run`) is acceptable.
3. **Protocol surface / version drift.** app-server is richer and more
   experimental than `codex exec`; method names and params (`thread/start`,
   `turn/start`, permission fields) should be pinned from
   `codex app-server generate-json-schema` for a specific codex version, and a
   minimum supported version documented.
4. **Approvals/sandbox fields.** The exact thread/turn fields for never-prompt +
   full access must be confirmed from the schema; getting them wrong means the
   agent stalls waiting on an approval that never comes (no human is attached).
5. **Operator output.** Forwarding `item/agentMessage/delta` gives readable
   progress; decide whether to also surface tool/command items or keep output
   minimal.
6. **Auth/`CODEX_HOME` provisioning.** The run environment must have codex auth;
   waap performs no startup validation, so misconfiguration surfaces only as a
   runtime `initialize`/`thread/start` error (agent left `running`).
