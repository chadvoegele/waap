# Add Pi as an agent-run system

Specification for adding `pi` to `waap agent run --system`, alongside
`opencode`, `claude`, and `codex`.

## Decision

WAAP should invoke the Pi Coding Agent directly through **Pi RPC mode**:

```text
waap (Rust) -> local `pi --mode rpc` child -> model provider
```

RPC mode is the right direct boundary because WAAP is written in Rust. It is
Pi's language-neutral process-integration API and provides:

- an authentic Pi session ID before work starts;
- correlated command responses and streamed lifecycle events;
- explicit prompt acceptance and abort commands;
- session persistence and future steering without scraping terminal output;
- process isolation from Pi's Node.js runtime.

The TypeScript SDK is preferable for Node.js applications, but using it from
WAAP would require a maintained Node sidecar and a second private protocol.
Print mode is too weak for controlled aborts. JSON event mode is one-way and
offers no advantage over RPC for a long-lived child.

## Goals

- Accept `waap agent run --agent-id <id> --system pi`.
- Run the agent in WAAP's existing per-agent worktree.
- Persist Pi's authentic session UUID in agent frontmatter before submitting the
  task prompt.
- Treat Pi's fully settled result as the backend outcome.
- Stream assistant text to the operator.
- Let `waap agent stop` abort the live Pi operation and preserve `aborted` state.
- Make interrupted Pi and Codex owners exit nonzero without terminal-state
  transition errors.
- Inherit the user's Pi auth, settings, extensions, skills, context files, and
  proxy configuration.
- Preserve the shared WAAP lifecycle, commits, reports, cleanup, and exit-code
  behavior.

## Non-goals

- Exposing Pi's full RPC surface through the WAAP CLI.
- Running Pi remotely over HTTP.
- Resuming or steering a completed WAAP agent.
- Adding a JavaScript runtime to the WAAP process.
- Defining a new sandbox or permission model. Pi and the current WAAP backends
  are trusted-user automation.

## Runtime preconditions

The Pi backend runs in the `waap` process's environment. The `pi` CLI must be
installed and authenticated, and the agent worktree and WAAP state directory
must be accessible at the paths passed to Pi.

## User interface and configuration

### System label

Add `AgentSystem::Pi` with the stable label `"pi"`. This enables:

```bash
waap agent run --agent-id aa-12345678 --system pi
```

and persisted frontmatter in the canonical
`${waap_data}/agents/<agent-id>/agent.md` record on WAAP's state branch:

```toml
system = "pi"
session_id = "019fdcce-5d74-7a54-8139-ce66c24dd93f"
```

The session ID is metadata, not part of the instruction body or agent worktree
copy. The default system remains `opencode`.

### Run options

Pi and Codex both accept the existing `--model` and `--reasoning-effort`
options; their help must no longer call them Codex-only. The selected backend
interprets each value:

```bash
waap agent run --agent-id aa-12345678 --system pi \
  --model openai-codex/gpt-5.6-sol --reasoning-effort high
```

Reject unsupported option/system combinations before changing agent state:

| Option | OpenCode | Claude | Codex | Pi |
| --- | --- | --- | --- | --- |
| `--model` | reject | reject | accept | accept |
| `--reasoning-effort` | reject | reject | accept | accept except `ultra` |

Reuse `ReasoningEffort`. Pi's child CLI calls the setting `--thinking`, so map
WAAP's `none` to Pi's `off`; map `minimal` through `max` directly; reject
`ultra` for Pi before changing agent state. Codex retains its existing values
and behavior.

### Environment

Use WAAP-prefixed variables to avoid confusing Pi's session metadata variables:

| Variable | Meaning |
| --- | --- |
| `WAAP_PI_BIN` | Pi executable; default `pi` |
| `WAAP_PI_MODEL` | Optional Pi model; overridden by `--model` |
| `WAAP_PI_REASONING_EFFORT` | Optional reasoning effort; CLI overrides it; translated to Pi `--thinking` |

Do not use `PI_MODEL` or `PI_REASONING_LEVEL` as configuration. Pi injects
those variables into commands to describe the current session, and WAAP may
itself be running inside a Pi session.

Before spawning the child, remove inherited parent-session metadata:

- `PI_SESSION_ID`
- `PI_SESSION_FILE`
- `PI_PROVIDER`
- `PI_MODEL`
- `PI_REASONING_LEVEL`

Preserve `HOME`, `PI_CODING_AGENT_DIR`, provider credentials, proxy variables,
CA settings, and the rest of the environment. Pi normally reads auth, settings,
models, extensions, skills, and sessions from `~/.pi/agent`; the
`PI_CODING_AGENT_DIR` override replaces that directory. Never place API keys on
the command line.

Invalid `WAAP_PI_MODEL`, `WAAP_PI_REASONING_EFFORT`, or CLI values must fail
before the agent enters `running`. Stop operations construct a
configuration-free Pi backend, as Codex stop does, so invalid run-only
configuration cannot prevent stopping an existing agent.

## Pi process invocation

Spawn one Pi child for one WAAP run:

```text
pi --mode rpc --approve --name "waap aa-12345678" \
  [--model <model>] [--thinking <level>]
```

Process configuration:

- `current_dir`: the agent worktree;
- stdin: piped;
- stdout: piped;
- stderr: inherited;
- session persistence: enabled; do not pass `--no-session`.

`--approve` deterministically trusts project-local Pi settings and resources
for this autonomous run. It is Pi project trust, not command-by-command
permission approval. The worktree remains the authority boundary already
chosen by WAAP.

The session name improves discovery in Pi's session list and provides operator
context. It is not used as the WAAP session identity.

## Backend design

Add `src/agent/pi.rs` with `PiBackend`, `PiRunConfig`, `PiRpcClient`, and
`PiRun`. Implement the existing `AgentSystemBackend` trait; do not move shared
WAAP lifecycle behavior into the backend.

### Start

`PiBackend::start(StartContext)` performs only session startup:

1. Install the same SIGTERM-to-interrupt flag used by direct process backends.
2. Spawn Pi in the worktree.
3. Start a dedicated stdout reader that applies strict LF-delimited JSONL
   framing and sends parsed records to the owning thread.
4. Send a correlated `get_state` command.
5. Read interleaved records until the matching successful response arrives.
6. Extract non-empty `data.sessionId`.
7. Return `StartedRun { session_id, handle }` without sending the task prompt.

Deferring the prompt is intentional. Shared orchestration persists the session
ID before calling `RunHandle::wait`, so an abort or persistence failure cannot
leave untracked Pi work running. This matches Codex's thread-before-turn
shape.

Pi also returns `sessionFile`; WAAP does not persist it. The session ID is Pi's
stable, resume-capable identity, and the session file remains discoverable
through Pi's session store.

### Wait

`PiRun::wait`:

1. Subscribe logically before submission by retaining all records received by
   the reader.
2. Send a correlated `prompt` command containing WAAP's existing common prompt.
3. Require a matching `success: true` response. This means accepted, not
   completed.
4. Pump events until `agent_settled`.
5. If the owner receives SIGTERM, send one correlated `abort` command and keep
   pumping until settlement or child exit.
6. Classify the final assistant message.
7. Close Pi stdin, wait for orderly child exit, and reap it.

Use `agent_settled`, not `agent_end`. `agent_end` may be followed by automatic
retry, overflow compaction, or queued continuation. `agent_settled` is Pi's
session-level completion boundary.

Outcome mapping:

| Final condition | `RunOutcome` |
| --- | --- |
| latest assistant `stopReason == "stop"` | `Completed` |
| `stopReason` is `error`, `length`, or `toolUse` | `Failed(1)` |
| `stopReason == "aborted"` or owner interrupt was requested | `Aborted` |
| settled without an assistant result | protocol `io::Error` |
| malformed JSON, failed RPC response, or EOF before settlement | protocol `io::Error` |

Add `RunOutcome::Aborted` to the shared backend outcome. Shared orchestration
maps it to exit code 1 and an idempotent `aborted` transition. Codex must map
`TurnStatus::Interrupted` to the same outcome instead of `Failed(1)`.

Tool-result errors alone do not fail the run; the model may recover and finish
normally. Unknown stop reasons are protocol errors rather than assumed success.

### Output

For `message_update` events whose `assistantMessageEvent.type` is
`text_delta`, write `delta` to stdout and flush. Do not emit thinking deltas or
raw RPC records. Pi stderr remains attached for startup and provider
diagnostics.

The client must still consume every record promptly to avoid backpressure.

### Extension UI requests

Project or global extensions can issue RPC UI requests. WAAP has no interactive
UI, so the backend must prevent them from hanging a run:

- respond with `{ "type": "extension_ui_response", "id": ..., "cancelled": true }`
  for `select`, `confirm`, `input`, and `editor`;
- ignore fire-and-forget `notify`, `setStatus`, `setWidget`, `setTitle`, and
  `set_editor_text` requests after optionally logging concise diagnostics.

Cancellation is the safe default for permission or confirmation extensions.

## RPC transport

Pi RPC uses strict LF-delimited JSONL:

- split only on byte `0x0a`;
- strip one trailing `\r` for CRLF input;
- preserve Unicode `U+2028` and `U+2029` inside JSON strings;
- serialize one JSON object plus `\n` per command;
- correlate responses by string request ID;
- process events and extension UI requests while awaiting any response.

A dedicated reader thread and an `mpsc` channel let the owner poll for the
SIGTERM flag while waiting for records. This avoids doing I/O in a signal
handler and avoids an indefinitely blocked `read_line` preventing abort.

Use a bounded startup/command-response timeout, defaulting to 30 seconds. Do
not impose an overall agent runtime timeout. EOF, reader failure, or child exit
must reject pending requests and include available stderr/process status in the
error without exposing credentials.

`PiRpcClient` must own and reap the child. On normal settlement, close stdin so
Pi disposes its runtime and exits. On startup failure or `PiRun` drop before
`wait`, close stdin, terminate if needed, and reap the child so session
persistence failures do not leak processes or zombies.

## Stop behavior

Pi's session ID identifies persisted conversation state but cannot attach a
second process to the live RPC stdin/stdout connection. `waap agent stop`
therefore follows the existing Codex owner-signal pattern:

1. The stop process resolves `system = "pi"` and calls `PiBackend::abort`.
2. `abort` sends SIGTERM to the live
   `waap agent run --agent-id <id>` owner process.
3. The owner's Pi run loop observes its signal flag and sends the RPC `abort`
   command over the connection it owns.
4. The stop process idempotently writes and commits WAAP's `aborted` state
   after signaling.
5. The owner observes Pi settlement, returns `RunOutcome::Aborted`, cleans the
   worktree, and exits 1 without reporting a transition error.

Use one shared idempotent aborted-transition helper from both stop orchestration
and `RunOutcome::Aborted` handling. If the record is already `aborted`, it is a
successful no-op with no extra commit. Thus either process may observe the
other's write without attempting `aborted -> failed` or `failed -> aborted`.
Apply the same outcome and transition behavior to Codex interruption.

Factor the argv-targeted signal and `pkill` status mapping out of `codex.rs` so
Codex and Pi use one tested helper. Exit statuses 0 (signaled) and 1 (already
exited) are accepted; other statuses are errors.

This retains the existing argv-matching limitation. Persisting owner PIDs or a
control socket would be a separate lifecycle change.

The start ordering narrows the existing sessionless-running race: Pi does not
receive the task prompt until the session ID has been committed. If stop marks
a sessionless run aborted, late session persistence fails and dropping the
handle shuts Pi down before task execution.

## Shared lifecycle

The existing shared sequence remains authoritative:

1. Validate that the agent is `ready`.
2. Validate Pi configuration and options.
3. Mark and commit `running` with `system = "pi"`.
4. Create the agent worktree.
5. Start Pi and obtain its session ID.
6. Persist and commit the session ID.
7. Submit the prompt and wait for settlement.
8. Clean the worktree.
9. Commit `completed` or `failed`, or idempotently converge on `aborted` for an
   interrupted run.

The Pi backend must not write agent records, create commits, choose worktree
paths, or update ticket state.

## Security and isolation

Pi runs with the privileges and resources of the `waap` process. These may
include the workspace, credential agents, password stores, container sockets,
browser services, and network credentials. RPC mode is process isolation, not
a security sandbox.

Operational requirements:

- never send secrets in argv, logs, frontmatter, or RPC diagnostics;
- inherit auth through Pi's credential store or provider environment;
- keep Pi local to the WAAP execution environment;
- do not expose an RPC port; communication is private stdio;
- cancel extension dialogs rather than auto-approving them;
- retain WAAP's isolated Git worktree.

## Compatibility

Target Pi Coding Agent `0.82.1` or newer, which provides the documented
`agent_settled` event, strict JSONL framing, `get_state`, prompt preflight
responses, and extension UI protocol used here.

WAAP should not parse Pi's session JSONL files to drive live state. Session
format is durable storage, while RPC events are the live protocol. A future Pi
protocol change should fail closed as a protocol error.

## Test strategy

### Unit tests

Use a fake Pi executable or child-process fixture that speaks the RPC protocol;
do not call a model provider.

Cover:

- command construction, cwd, inherited environment, and parent `PI_*` removal;
- CLI-over-environment model and reasoning-effort precedence, including Pi value mapping;
- invalid Pi options failing before `running`;
- `get_state` session extraction before prompt submission;
- shared session persistence occurring before `prompt` is written;
- interleaved events and correlated responses;
- strict LF framing with CRLF, chunked UTF-8, `U+2028`, and `U+2029`;
- assistant text forwarding without duplicate final text;
- completion only after `agent_settled`, including an `agent_end` followed by
  retry events;
- every stop-reason mapping;
- prompt rejection, malformed JSON, unknown status, reader failure, timeout,
  and premature EOF;
- cancellation responses for every dialog-style extension UI request;
- fire-and-forget extension UI requests not blocking;
- SIGTERM causing exactly one RPC `abort` command;
- Pi and Codex interruption converging on `aborted` in either process order,
  with owner exit 1 and no transition error;
- child shutdown and reaping after success, error, and dropped handles.

Extend backend/lifecycle tests to include `AgentSystem::Pi` for:

- enum parsing, labels, and frontmatter validation;
- backend construction and type selection;
- common prompt and worktree context;
- session commit ordering;
- completed, failed, errored, and concurrently aborted outcomes;
- mixed-system stop dispatch.

### Integration smoke test

With an authenticated Pi installation and a disposable repository:

1. create a WAAP agent that writes and commits a small file;
2. run it with `--system pi`;
3. verify running, session, and completed commits;
4. verify the Pi session appears under the worktree cwd;
5. verify the worktree is removed;
6. repeat with a long-running prompt, call `waap agent stop`, and verify the
   final state is `aborted`, Pi exits, and the worktree is removed.

The provider-backed smoke test is manual or opt-in and must not run in the
default test suite.

## Affected files

- `src/agent.rs`: add `AgentSystem::Pi`, Pi option validation, and backend
  construction.
- `src/agent/pi.rs`: add Pi configuration, RPC client, run handle, event
  classification, and abort implementation.
- `src/agent/backend.rs`: add `RunOutcome::Aborted` and host a shared
  owner-signal helper if factored from Codex.
- `src/agent/codex.rs`: reuse the shared signal helper and map interrupted turns
  to `RunOutcome::Aborted`.
- `src/agent/run.rs`: handle aborted outcomes idempotently and return exit 1
  without a transition error.
- `src/agent/stop.rs`: reuse the idempotent aborted transition.
- `src/cli.rs`: update `--model` and `--reasoning-effort` help and ownership.
- `README.md` and `.agents/skills/waap/SKILL.md`: document the new system and
  run options.
- `Cargo.toml`: no async runtime is required; use the standard library,
  `serde_json`, and existing `signal-hook`.

## Rejected alternatives

### Embed the TypeScript SDK

Rejected because WAAP is Rust. A Node bridge would recreate RPC with an
additional package and protocol to version. Pi's own documentation recommends
RPC for non-Node integrations.

### Use print mode

Rejected because it does not expose a live control channel, prompt acceptance,
or extension UI handling.

### Use JSON event mode

Rejected because it is one-way. It can stream output but cannot issue a
protocol-level abort or support future control operations.

### Parse session files for completion

Rejected because session JSONL is persistence, not a live synchronization
protocol. Polling it introduces races and couples WAAP to storage internals.

### Treat process exit zero as task success

Rejected because the RPC process is a session host and normally remains alive
until stdin closes. Agent completion and model errors are represented by events
and assistant stop reasons, not by child exit alone.

### Run the prompt during `start`

Rejected because shared orchestration cannot persist the returned session ID
until `start` returns. Deferring the prompt to `wait` makes the durable session
commit a precondition for task execution.

## Implementation checklist

- [ ] Add `pi` to CLI and frontmatter system values without changing the
      default.
- [ ] Accept shared model/reasoning options, map Pi values, and validate
      option ownership.
- [ ] Spawn direct `pi --mode rpc` in the agent worktree.
- [ ] Implement strict LF JSONL parsing and correlated responses.
- [ ] Obtain and persist `get_state.data.sessionId` before prompting.
- [ ] Cancel extension dialogs and ignore fire-and-forget UI safely.
- [ ] Wait for `agent_settled` and map the final assistant stop reason.
- [ ] Forward assistant text deltas.
- [ ] Implement owner-signal-to-RPC-abort behavior and child reaping.
- [ ] Add idempotent aborted outcomes for Pi and Codex without transition
      errors.
- [ ] Preserve shared state transitions, commits, reports, and cleanup.
- [ ] Add fake-process protocol and lifecycle tests.
- [ ] Update user documentation and the WAAP skill.
- [ ] Run `cargo fmt --check`.
- [ ] Run `cargo clippy --all-targets -- -D warnings`.
- [ ] Run `cargo build` and `cargo build --release`.
- [ ] Run `cargo test` outside the command sandbox.
- [ ] Run `waap check`.
