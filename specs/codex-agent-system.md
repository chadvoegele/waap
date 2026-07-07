# Add `codex` as an agent-run system

Implementation plan for adding `codex` as a third value of `waap agent run
--system`, alongside `opencode` and `claude`.

## Design

`codex` is driven through **`codex app-server --stdio`**, a JSON-RPC 2.0 server
spawned as a per-run child process. Within a run:

- `session_id` is codex's authentic `ThreadId`, returned synchronously by
`thread/start`.
- The agent runs with never-prompt approvals and full sandbox access.
- The agent's event stream is forwarded to the operator's stdout.
- Completion is derived from the `turn/completed` status.
- `waap agent stop` signals the run process, which issues a graceful
`turn/interrupt`.

## codex app-server protocol

`codex app-server` (alias `--stdio`) speaks JSON-RPC 2.0 over stdin/stdout as
newline-delimited JSON (the `"jsonrpc":"2.0"` header is omitted on the wire).
One connection per process; the process exits when the connection closes.
Methods waap uses:

- **`initialize`** (with client metadata/capabilities), followed by the
`initialized` notification. Any other request before this handshake is
rejected.
- **`thread/start`** — params include `cwd`, permission/sandbox overrides,
optional `model`. The response returns the thread object including its
`ThreadId`; the server also emits `thread/started` and auto-subscribes the
connection to that thread's turn/item events.
- **`turn/start`** — `{ threadId, input, … }`; returns the new turn object
(with a turn id), emits `turn/started`, then streams `item/started`,
`item/completed`, `item/agentMessage/delta`, and tool-progress notifications.
- **`turn/completed`** — sent with the final turn state (status + token usage)
when the model finishes or the turn is interrupted.
- **`turn/interrupt { thread_id, turn_id }`** → `{}`; the turn ends with
`status: "interrupted"`.

## 1. `AgentSystem::Codex` variant and CLI wiring

In `src/agent.rs`: add `Codex` to `AgentSystem`; `as_str` ⇒ `"codex"`. `parse`
and `labels` need no change (they iterate `value_variants()`), so frontmatter
`system = "codex"` validates and `--system codex` parses automatically.

In `src/cli.rs`: no structural change — `--system` is a `value_enum` over
`AgentSystem`.

## 2. JSON-RPC app-server client (`src/agent/codex.rs`)

Add `src/agent/codex.rs` as the client to the app-server, over a spawned stdio child.

Spawn command (no prompt on the argv — the prompt is sent as turn input),
launched with `current_dir = <worktree>`:

```
codex app-server --stdio
```

Config:

```rust
pub(crate) struct CodexRunConfig {
    pub(crate) model: Option<String>,   // from CODEX_MODEL (optional)
    pub(crate) repo_root: PathBuf,       // set to the worktree at run time
}
```

A minimal JSON-RPC client over the child's stdin/stdout:

- write framed requests/notifications (newline-delimited JSON, no `jsonrpc`
header), correlate responses by `id`, and dispatch inbound notifications;
- typed for the methods waap uses: `initialize`, `initialized`, `thread/start`,
`turn/start`, `turn/interrupt`, and inbound `turn/completed` +
`item/agentMessage/delta`;
- configure `thread/start` (and/or `turn/start`) for never-prompt approvals and
full sandbox access.

## 3. Driving a run (`run_agent_codex` in `src/agent/run.rs`)

codex does not reuse `run_forwarding`. Add `run_agent_codex`, modeled
structurally on `run_agent_opencode`:

```rust
fn run_agent_codex(repo_root, output_format, agent_id) -> io::Result<ExitCode> {
    let mut config = codex_run_config_from_env(repo_root)?;

    let (mut metadata, body) = read_agent_record(repo_root, agent_id)?;
    metadata.system = Some(AgentSystem::Codex);
    // session_id (ThreadId) is unknown until thread/start returns.

    let outcome = run_in_agent_worktree(
        repo_root, agent_id,
        || mark_running(repo_root, output_format, agent_id, &mut metadata, &body),
        |worktree| {
            config.repo_root = worktree.to_path_buf();
            let mut client = spawn_codex_app_server(&config)?;   // child + JSON-RPC
            client.initialize()?;
            let thread_id = client.thread_start(worktree)?;       // real id, synchronous

            // Persist the authentic ThreadId as session_id, then commit.
            update_codex_session(repo_root, output_format, agent_id, &thread_id)?;

            let prompt = format!(
                "Complete when instructions in /.waap/agents/{agent_id}/agent.md are satisfied"
            );
            let turn_id = client.turn_start(&thread_id, &prompt)?;
            // Forward agentMessage deltas to waap stdout; return the final turn status.
            client.pump_until_turn_completed(&thread_id, &turn_id)   // -> TurnStatus
        },
    )?;

    finalize_codex_run(repo_root, output_format, agent_id, outcome)
}
```

Extend the `run_agent` dispatch with `AgentSystem::Codex =>
run_agent_codex(...)`. The worktree lifecycle (`run_in_agent_worktree`,
`mark_running`) is reused verbatim.

`mark_running` commits `running` before the worktree is cut, so `session_id` is
not known yet; `thread/start` returns the `ThreadId` once the server is up
inside the worktree, and `update_codex_session` writes it and commits (one
extra commit per codex run). `session_id` already exists on `AgentMetadata` —
no schema change.

`run_agent_codex` installs a `SIGTERM` handler that calls
`turn/interrupt(thread_id, turn_id)` and closes the connection (see §5).

### Completion

claude/opencode derive completion from a process exit code via
`finalize_agent_run`. codex derives it from the `turn/completed` status:
`Completed` ⇒ success (mark agent `completed`, exit 0); `Failed`/`Interrupted`
⇒ leave `running`, return a non-zero `ExitCode`. `finalize_codex_run` applies
the same mark/commit logic as `finalize_agent_run`/`mark_completed`, keyed on
the turn status instead of an `ExitStatus` (alternatively, refactor
`finalize_agent_run` to take a `success: bool`).

## 4. Session id

`session_id` = codex's authentic `ThreadId`, taken directly from the
`thread/start` response — the genuine, resume-capable id (usable later with
`thread/resume`/`thread/fork`).

## 5. `waap agent stop`

`turn/interrupt` requires the live JSON-RPC connection, held only by the running
`waap agent run` process (R). `waap agent stop` therefore signals R and lets R
interrupt gracefully:

- `waap agent stop` sends `SIGTERM` to R, matched by R's unique argv: `pkill
-TERM -f "agent run --agent-id <agent-id>"`. This matches R, not the `codex
app-server --stdio` child (which lacks the agent id), and is independent of
whether R runs in the foreground or backgrounded (`nohup`/`setsid`).
- R's `SIGTERM` handler calls `turn/interrupt(thread_id, turn_id)`, closes the
connection, and returns through `run_in_agent_worktree` so the worktree is
cleaned up. The interrupted turn yields a non-`Completed` status, so
`finalize_codex_run` leaves the agent `running` and never overwrites the
`aborted` status `waap agent stop` writes to the record.
- In `src/agent/stop.rs::stop_agents_with_systems`, the `AgentSystem::Codex`
arm needs the **agent id** (available in `stop_agent_if_running`), not the
`session_id`, so the abort closure signature passes `agent_id`. This is the one
place codex diverges from the claude/opencode `abort(system, session_id)`
shape.

Because a stdio server exits when its stdin EOFs, the child app-server is torn
down automatically if R dies for any reason; signalling R is the only stop path
waap implements.

## 6. Worktree integration

`run_in_agent_worktree` cuts `worktrees/<agent-id>` from the `running` commit,
runs inside it, and removes it afterward (even on error). codex reuses this
unchanged: spawn `codex app-server --stdio` with `current_dir = worktree` and
pass that `cwd` to `thread/start` so the model's tools operate in the worktree.
`CODEX_HOME` (auth/config/sessions, default `~/.codex`) is inherited from the
environment, is not the worktree, and is neither set nor relocated by waap.

## 7. Config / env

- **Model:** `CODEX_MODEL`, mirroring `CLAUDE_MODEL`.
`codex_run_config_from_env` reads it with
`env::var("CODEX_MODEL").ok().filter(|m| !m.is_empty())`; when set, pass it as
the `model` field on `thread/start`/`turn/start`, else use codex's default.
- **Auth:** codex auth (API key or prior `codex login`) is an operator
precondition for `--system codex`, as claude assumes its own auth.
`codex_run_config_from_env` has no required vars, so it never fails for missing
config; a misconfigured environment surfaces as an `initialize`/`thread/start`
error and the agent is left `running`.
