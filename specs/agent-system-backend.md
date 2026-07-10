# Agent system backend

Design for consolidating Opencode, Claude, and Codex behavior behind one
system abstraction while retaining the existing run and stop lifecycle.

## Scope and baseline

This design is based on the code after these blocking tickets were completed:

- `tt-refactor-agent-run-cleanup-and-completion`
- `tt-reject-already-running-agent-runs`
- `tt-trim-agent-run-comments`
- `tt-simplify-codex-json-rpc-client`

The implementation must preserve CLI syntax, frontmatter, commits, reports,
exit codes, worktree ordering and cleanup, session timing, stop behavior, and
error propagation. It does not fix adjacent lifecycle races or rerun policy.

## Decision

Retain `AgentSystem` as the CLI and persisted enum. Add a separate synchronous,
object-safe `AgentSystemBackend` trait for behavior and a command-scoped
`BackendRegistry` that maps the enum to a lazily constructed implementation.

This separates stable data identity from runtime behavior:

- `AgentSystem` remains serializable, accepted by Clap, and compatible with
  existing `system = "..."` frontmatter.
- `AgentSystemBackend` owns only system-specific preparation, execution, and
  abort behavior.
- `run.rs` and `stop.rs` retain lifecycle orchestration.
- `BackendRegistry::resolve` is the only production behavioral dispatch from
  `AgentSystem` to an implementation. Enum label matches such as `as_str`
  remain data conversion, not backend dispatch.
- Tests replace the registry through a small resolver trait and use fake
  backends without HTTP requests, child processes, or process signals.

No async runtime, new dependency, or public API is needed.

## Proposed types

Add `src/agent/backend.rs` with the shared behavioral boundary:

```rust
use std::io;
use std::path::Path;
use std::process::ExitCode;

use super::AgentSystem;

pub(super) struct RunPreparation {
    pub(super) initial_session_id: Option<String>,
}

pub(super) enum RunOutcome {
    Completed,
    Failed(ExitCode),
}

pub(super) struct RunContext<'a> {
    pub(super) agent_id: &'a str,
    pub(super) prompt: &'a str,
    pub(super) initial_session_id: Option<&'a str>,
    pub(super) worktree_dir: &'a Path,
    pub(super) publish_session:
        &'a mut dyn FnMut(&str) -> io::Result<()>,
}

pub(super) struct AbortContext<'a> {
    pub(super) waap_root: &'a Path,
    pub(super) agent_id: &'a str,
    pub(super) session_id: &'a str,
}

pub(super) trait AgentSystemBackend {
    fn prepare_run(&mut self) -> io::Result<RunPreparation>;

    fn run(&mut self, context: RunContext<'_>)
        -> io::Result<RunOutcome>;

    fn abort(&mut self, context: AbortContext<'_>) -> io::Result<()>;
}

pub(super) trait BackendResolver {
    fn resolve(
        &mut self,
        system: &AgentSystem,
    ) -> io::Result<&mut (dyn AgentSystemBackend + '_)>;
}
```

`RunPreparation` represents only information required before the shared
`running` transition. It does not start a process or create a worktree. Claude
returns its generated session ID here. Opencode and Codex return `None` because
their authentic IDs do not exist yet. Codex also installs its SIGTERM handler
and retains the interrupt flag in its backend during this phase.

`RunContext::publish_session` is an event back into shared orchestration, not a
backend-selection callback. Opencode invokes it immediately after `/session`
returns; Codex invokes it immediately after `thread/start`. This persists the
ID before either long-running operation continues. Claude does not invoke it
because its initial ID is included in the `running` commit.

`RunOutcome` normalizes only the lifecycle decision:

- `Completed` causes shared orchestration to mark and commit `completed` and
  return `ExitCode::SUCCESS`.
- `Failed(code)` leaves the record unchanged and returns that code.
- `io::Error` preserves infrastructure/protocol errors and participates in the
  existing run/cleanup error collapse.

Opencode and Claude convert a successful `ExitStatus` to `Completed` and a
non-success status to `Failed(exit_code_from_status(status))`. Codex maps
`TurnStatus::Completed` to `Completed` and all other statuses to
`Failed(ExitCode::FAILURE)`. Raw process and protocol result types do not leak
into shared orchestration.

Both contexts deliberately include the superset of stable inputs needed by the
three implementations. Opencode abort uses WAAP root, agent ID, and session ID;
Claude uses session ID; Codex uses agent ID. Opencode alone derives and
canonicalizes `worktrees/<agent-id>` inside its abort method. Merely stopping a
Claude or Codex run therefore does not gain a fallible filesystem operation.
These differences remain visible without adding system matches outside the
registry.

## Ownership and lifetimes

The production registry owns configured backends for one CLI command:

```rust
#[derive(Default)]
pub(super) struct BackendRegistry {
    opencode: Option<OpencodeBackend>,
    claude: Option<ClaudeBackend>,
    codex: Option<CodexBackend>,
}

impl BackendResolver for BackendRegistry {
    fn resolve(
        &mut self,
        system: &AgentSystem,
    ) -> io::Result<&mut (dyn AgentSystemBackend + '_)> {
        match system {
            AgentSystem::Opencode => {
                if self.opencode.is_none() {
                    self.opencode = Some(OpencodeBackend::from_env()?);
                }
                Ok(self.opencode.as_mut().expect("initialized"))
            }
            AgentSystem::Claude => {
                if self.claude.is_none() {
                    self.claude = Some(ClaudeBackend::from_env());
                }
                Ok(self.claude.as_mut().expect("initialized"))
            }
            AgentSystem::Codex => {
                if self.codex.is_none() {
                    self.codex = Some(CodexBackend::from_env());
                }
                Ok(self.codex.as_mut().expect("initialized"))
            }
        }
    }
}
```

The exact initialization can use small private helpers rather than repeating
the `Option` code, but the enum match remains in this method.

- Each backend owns its configuration. No configuration or credentials are
  cloned into contexts.
- The registry lives on the stack in `run_agent` or
  `stop_agents_with_systems`. It is neither global nor shared between commands.
- `resolve` returns a mutable trait-object borrow tied to `&mut self`; no
  backend or context requires `'static`, `Send`, or `Sync`.
- Run and abort contexts borrow IDs and paths only for the method call and
  cannot outlive orchestration state.
- `publish_session` mutably borrows the session-persistence closure only during
  `run`.
- Child processes, HTTP clients, and the Codex JSON-RPC client remain local to
  their backend method. The trait does not erase or transfer their ownership.
- `CodexBackend` owns `Option<Arc<AtomicBool>>` between `prepare_run` and
  `run`; `run` takes the prepared flag and errors if preparation was skipped.
  A fresh flag is installed for each preparation.

`&mut self` permits Codex's prepared state and recording fakes without interior
mutability. Runs are synchronous and a command executes one backend method at a
time, so a more complex concurrency contract is unnecessary.

## Configuration

Configuration is loaded when `BackendRegistry::resolve` first selects a
system, not when the registry is created:

| Backend | Construction | Retained state |
| --- | --- | --- |
| Opencode | Fallible `from_env()` reads the four required `OPENCODE_SERVER_*` variables. | `OpencodeRunConfig` for all selected operations in the command. |
| Claude | Infallible `from_env()` reads optional `CLAUDE_MODEL`. | `ClaudeRunConfig`. |
| Codex | Infallible `from_env()` reads optional `CODEX_MODEL`. | `CodexRunConfig` plus a per-run interrupt flag after preparation. |

This preserves lazy Opencode configuration during stop-all. Stopping only
Claude or Codex agents does not read or require Opencode variables. Multiple
Opencode aborts in one stop-all command reuse one loaded configuration, as they
do now. A running record with no session is marked aborted without resolving a
backend, so it also does not require configuration.

For a run, the selected backend is resolved before `mark_running`; missing
Opencode configuration therefore still fails without changing agent state.

## Shared run orchestration

Keep one production wrapper and one injectable implementation in `run.rs`:

```rust
pub(crate) fn run_agent(
    waap_root: &Path,
    output_format: &OutputFormat,
    agent_id: &str,
    system: &AgentSystem,
) -> io::Result<ExitCode> {
    let mut backends = BackendRegistry::default();
    run_agent_with_backends(
        waap_root,
        output_format,
        agent_id,
        system,
        &mut backends,
    )
}
```

`run_agent_with_backends` performs this sequence:

1. Read the record and reject an already-running agent before backend
   resolution.
2. Resolve only the selected backend.
3. Re-read the record, matching the second read currently performed by each
   system-specific runner.
4. Call `prepare_run`. UUID generation and Codex signal-hook errors occur after
   that read but before state changes. In particular, Codex does not install a
   process-global handler if the second read fails.
5. Set `metadata.system` to the selected enum, and replace
   `metadata.session_id` only when preparation returned an initial ID. This
   preserves current Claude behavior and current stale-session behavior for
   Opencode/Codex.
6. Call the existing `mark_running`, including its immediate duplicate check,
   record write, commit, report, and output.
7. Create `AgentWorktree` after the running commit.
8. Build the common prompt once and call `backend.run` with the worktree,
   initial session, and a publisher that delegates to
   `update_agent_session(..., system.clone())`.
9. Explicitly clean the worktree and combine run and cleanup errors through
   `collapse_errors`. `Drop` remains the fallback for early exits.
10. On `RunOutcome::Completed`, call `mark_completed`; on `Failed(code)`, leave
   the latest status unchanged and return `code`.

The following remain shared and do not move into a backend:

- record reads and `running`/`completed` transitions;
- duplicate-running checks;
- frontmatter writes;
- commits, report loading, and output formatting;
- worktree path, creation after the running commit, explicit cleanup, and Drop
  cleanup;
- late session persistence and its commit;
- common prompt construction;
- run/cleanup error precedence;
- conversion from normalized outcome to CLI exit code.

This removes `run_agent_opencode`, `run_agent_claude`, and `run_agent_codex`
from `run.rs`. Their protocol/process bodies move behind backend implementations
without moving shared lifecycle operations.

## Backend run mapping

### Opencode

`OpencodeBackend` owns `OpencodeRunConfig`.

- `prepare_run`: return no initial session.
- `run`: create the remote session for `worktree_dir`, publish the returned ID,
  build and spawn `opencode run`, wait, and map `ExitStatus` to `RunOutcome`.
- `abort`: canonicalize `waap_root`, derive `worktrees/<agent-id>`, and call
  `/session/<session_id>/abort` with that directory.

Session publication remains after worktree creation and before CLI spawn. A
remote session followed by a persistence failure remains an error, as today.

### Claude

`ClaudeBackend` owns `ClaudeRunConfig`.

- `prepare_run`: generate and return `Uuid::new_v4()` as the initial session.
- `run`: require `initial_session_id`, build and spawn `claude -p`, wait, and
  map `ExitStatus` to `RunOutcome`.
- `abort`: call `pkill -TERM -f <session_id>` through the existing helper.

The UUID remains in the initial running commit. Claude does not publish a late
session.

### Codex

`CodexBackend` owns `CodexRunConfig` and prepared interrupt state.

- `prepare_run`: create `Arc<AtomicBool>`, register the SIGTERM flag handler,
  retain the flag, and return no initial session.
- `run`: take the prepared flag; spawn and initialize the app server; start a
  thread; publish its authentic thread ID; start the turn with the common
  prompt; pump notifications; and map `TurnStatus` to `RunOutcome`.
- `abort`: signal the live `waap agent run --agent-id <agent-id>` process
  through the existing helper.

Codex interruption is intentionally not modeled as a direct session abort.
Only the live run process owns the JSON-RPC connection. The stop process sends
SIGTERM; the prepared flag causes the live backend's pump to issue
`turn/interrupt`. An interrupted outcome does not call `mark_completed`, so it
does not overwrite the `aborted` status written by stop.

## Shared stop orchestration

Replace the low-level abort closure with `&mut dyn BackendResolver`:

```rust
fn stop_agents(
    waap_root: &Path,
    agent_id: Option<&str>,
    backends: &mut dyn BackendResolver,
) -> io::Result<Vec<AgentReport>>;
```

`stop_agents_with_systems` creates one `BackendRegistry`, calls `stop_agents`,
then retains the existing combined commit and report behavior.

For each running record, `stop_agent_if_running` keeps the current order:

1. Load and verify running status.
2. Re-read metadata and body.
3. If a session exists, select persisted `metadata.system`, defaulting a legacy
   missing value to `AgentSystem::Opencode`.
4. Resolve that backend and call `abort` with WAAP root, agent ID, and session
   ID. Only Opencode derives and canonicalizes its worktree path.
5. Only after successful abort, write `aborted` and reload the report.

Sessionless records skip resolution and abort. Stop-all remains sequential and
retains one registry, so lazy configuration is reused. The combined commit is
still created only after all selected records are processed.

## Test strategy

Add a test-only `FakeBackend` implementing the same trait. It records copied
values from borrowed contexts and has configurable preparation, late session,
outcome, run error, and abort error. A `FakeResolver` owns one fake per system
or records each requested enum before returning a selected fake.

Run orchestration tests should prove:

- the requested enum is resolved once and persisted;
- backend resolution/configuration and preparation fail before `running`;
- an initial session is included in the running commit;
- no initial session leaves metadata empty until publication;
- a late publication writes and commits the authentic session before run
  completion;
- the backend receives the created worktree, prompt, agent ID, and initial
  session;
- `Completed` cleans the worktree and commits `completed`;
- `Failed(code)` cleans the worktree, preserves non-completed status, and
  returns the exact code;
- backend errors clean up the worktree and propagate;
- an already-running record is rejected without resolving a backend.

Retain focused `collapse_errors` unit tests for every run/cleanup result
combination and existing `AgentWorktree` creation, explicit-cleanup, and Drop
tests. Do not add a worktree abstraction solely to force cleanup failures
through fake-backed orchestration.

Stop orchestration tests should replace callback assertions with fake-backend
assertions and prove:

- mixed Opencode, Claude, and Codex records resolve the correct backend;
- abort receives WAAP root, agent ID, and session ID;
- missing `system` resolves Opencode for legacy records;
- sessionless running records do not resolve a backend;
- abort or resolver failure leaves that record running;
- non-running records are skipped;
- stop-one, stop-all ordering, combined commit, and output remain unchanged.

Keep adapter-level tests for command construction, exit-status mapping,
Opencode payload/query and worktree-path construction, Codex JSON-RPC framing
and notifications, and `pkill` status handling. Extract Claude's inline
`pkill` exit mapping into a private helper and test exit codes 0, 1, other
nonzero values, and signal termination, matching Codex's existing coverage.
Backend-orchestration fakes are not a reason to introduce HTTP or process-runner
traits. Add focused backend tests only where the wrapper has nontrivial mapping,
especially Claude's required initial session and Codex's preparation
requirement.

## Migration sequence

Keep each step compiling and avoid unrelated adapter cleanup:

1. Add `backend.rs` with the contexts, preparation/outcome types, traits, and
   empty lazy registry shape. Add test-only fake backend and resolver helpers.
   Existing dispatch remains active.
2. Add `OpencodeBackend`, `ClaudeBackend`, and `CodexBackend` wrappers around
   current helper functions. Add focused outcome, preparation, and abort-status
   mapping tests. Do not move lifecycle code yet.
3. Populate `BackendRegistry::resolve` as the sole behavioral enum dispatch.
   Verify backend construction is lazy, fallible only for Opencode, and reused
   within one registry.
4. Introduce `run_agent_with_backends` beside the current system-specific run
   functions and cover it with fakes. Then switch the production `run_agent`
   wrapper to the registry and delete the three old runner functions. Keep
   `mark_running`, `update_agent_session`, `mark_completed`, `AgentWorktree`,
   and `collapse_errors` in place to minimize the diff.
5. Change `stop_agents` and `stop_agent_if_running` from the abort closure to
   `BackendResolver`, migrate existing callback tests to fakes, and reduce
   `stop_agents_with_systems` to registry construction plus the existing commit
   and report logic. Remove its enum match and Opencode config cache only after
   equivalent lazy-registry tests pass.
6. Remove adapter helper visibility that became unnecessary, run the complete
   validation suite, and inspect the final diff for any production backend
   dispatch outside `BackendRegistry::resolve` or any lifecycle operation
   inside a backend.

This order establishes behavior and tests before deleting either dispatch path,
and it does not require a broad module or application-layer rewrite.

## Affected files

- `src/agent.rs`: declare `backend`; retain `AgentSystem` and its persisted/CLI
  behavior unchanged.
- `src/agent/backend.rs`: add contexts, preparation/outcome types, behavioral
  and resolver traits, and the lazy production registry.
- `src/agent/run.rs`: replace three-way dispatch and system-specific run
  functions with shared injectable orchestration; retain lifecycle helpers and
  `AgentWorktree`.
- `src/agent/stop.rs`: replace the abort callback and duplicate enum match with
  resolver-backed abort; retain record selection, transitions, combined commit,
  reporting, and legacy Opencode default.
- `src/agent/opencode.rs`: add `OpencodeBackend` and its trait implementation
  around existing HTTP, command, and spawn helpers.
- `src/agent/claude.rs`: add `ClaudeBackend` and its trait implementation around
  existing config, command, spawn, and kill helpers.
- `src/agent/codex.rs`: add `CodexBackend`, retain the interrupt flag between
  preparation and run, and wrap the existing app-server client and signal
  helper.
- `tests/state_commits.rs`: update or add lifecycle integration assertions only
  if unit tests cannot cover an observable commit/report contract.

`src/app.rs`, `src/cli.rs`, frontmatter schema, and dependencies should not need
production changes. Their existing tests remain compatibility coverage.

## Rejected alternatives

### Replace `AgentSystem` with trait objects

Rejected because the enum is persisted data and a Clap value. Trait objects do
not provide stable labels or serialization. Keeping the enum also preserves
legacy defaulting and validation.

### Put the full run lifecycle in each backend

Rejected because it would retain or increase duplication of record transitions,
commits, worktree cleanup, reporting, and completion. Those are WAAP lifecycle
rules, not system behavior.

### One undifferentiated `run` method with no preparation

Rejected because Claude's session must be persisted in the running commit and
Codex's signal handler must be installed before that transition. Opencode and
Codex cannot provide authentic sessions at the same time as Claude.

### Return a session only when `run` finishes

Rejected because Opencode and Codex sessions must be durable while the run is
live so stop and status commands can use them.

### Eagerly construct every backend

Rejected because it would make Claude/Codex stop operations require unrelated
Opencode environment variables and would load secrets unnecessarily.

### Construct a fresh backend for every stopped agent

Rejected because stop-all currently loads Opencode configuration once. A lazy
command-scoped registry preserves that behavior with little additional state.

### Static generic dispatch or an enum wrapper only

Rejected because stop-all handles a runtime mixture of persisted systems, and
tests need to replace behavior without propagating generic parameters through
the application. An enum wrapper would centralize the match but still couple
tests to concrete HTTP and process behavior unless it recreated a trait seam.

### Separate run and abort backend traits

Rejected for now because all supported systems implement both operations and
share configuration. One small trait keeps system behavior discoverable in one
place. Split it only if a future backend genuinely supports one capability.

### Async methods or low-level HTTP/process abstractions

Rejected because all current orchestration is blocking and command-scoped.
Named backend behavior is the missing test seam; abstracting transports would
add churn without improving lifecycle tests.

## Risks and preserved limitations

- OpenCode and Codex have a sessionless running window. Stop can mark such a
  record aborted without contacting the backend while the run continues. This
  design preserves that existing race; changing it requires a lifecycle ticket.
- Rerunning records with existing sessions remains inconsistent: Claude replaces
  its session, while late Opencode/Codex publication rejects an existing ID.
  Existing worktree branch reuse can also fail. This is out of scope.
- The duplicate-running checks remain non-atomic. Backend resolution and
  preparation stay before the second check, matching current ordering as
  closely as possible.
- Stop-all still writes earlier aborted records before a later abort/config
  failure and commits only after the loop. The refactor must not claim
  transactional stop semantics.
- Codex abort still depends on argv matching and on the live process converting
  SIGTERM into a JSON-RPC interrupt. The trait must not hide this as a direct
  session operation.
- Codex signal hooks are process-global. Tests should not register production
  hooks in parallel; fake preparation should cover orchestration.
- A backend borrow from the registry spans `run`, including the late-session
  callback. The callback must not attempt to resolve another backend. Current
  session persistence does not do so.
- Moving exit mapping can accidentally turn a backend failure into an
  `io::Error` or lose a child exit code. `RunOutcome` mapping needs direct tests.
- Moving worktree or session operations into implementations would weaken
  cleanup and commit-order guarantees. Code review should reject such drift.

## Implementation and validation checklist

- [ ] Add the backend types, trait, resolver, and lazy registry without changing
      `AgentSystem` labels or frontmatter behavior.
- [ ] Implement Opencode, Claude, and Codex backends with the session and config
      timing described above.
- [ ] Consolidate run orchestration and remove all production behavioral
      system dispatch except `BackendRegistry::resolve`.
- [ ] Consolidate stop dispatch through the same resolver and preserve legacy
      Opencode defaulting.
- [ ] Keep record transitions, commits/reports, worktree ownership/cleanup,
      session persistence, completion, and error collapsing outside backends.
- [ ] Add fake-backed run lifecycle tests and migrate stop callback tests.
- [ ] Retain adapter protocol, command, payload, and signal-status tests.
- [ ] Verify no production CLI, frontmatter, report, commit-message, exit-code,
      or lifecycle behavior changes.
- [ ] Run `cargo run -- check`.
- [ ] Run `waap check`.
- [ ] Run `cargo fmt --check`.
- [ ] Run `cargo clippy --all-targets -- -D warnings`.
- [ ] Run `cargo build`.
- [ ] Run `cargo build --release`.
- [ ] Run `cargo test` outside any command sandbox.
