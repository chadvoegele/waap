# Plan: add `codex` as a third agent-run system

## Status

Planning only. This document is the implementation plan for adding `codex` as a
third value of `waap agent run --system`, alongside the existing `opencode` and
`claude`. **No `src/` behavior is changed by the ticket that produces this
document.** Each section below names the concrete `codex exec` flags and the
concrete waap functions/files an implementer will touch.

## Background: how the existing systems are wired

`waap agent run --system <opencode|claude>` runs a coding agent in a freshly cut
git worktree and derives the agent's terminal status from the process exit code.
The relevant pieces (all under `src/`):

- `src/agent.rs` — `AgentSystem` enum (`Opencode`, `Claude`) with
  `as_str`/`parse`/`labels`, persisted to/from the agent record frontmatter
  (`system = "..."`).
- `src/cli.rs` — `AgentCommand::Run { agent_id, system }`; `system` is a clap
  `value_enum` defaulting to `opencode`.
- `src/claude.rs` — `ClaudeRunConfig`/`ClaudeRunCommand`,
  `claude_run_config_from_env` (reads `CLAUDE_MODEL`),
  `build_claude_run_command`, `run_claude_attached` (forwards stdout/stderr and
  fires an `on_started` hook), `kill_claude_session`
  (`pkill -TERM -f <session_id>`).
- `src/opencode.rs` — the opencode equivalent, including HTTP session
  creation/abort.
- `src/agent/run.rs` — `run_agent` dispatch, `run_agent_claude`/
  `run_agent_opencode`, `run_in_agent_worktree` (worktree lifecycle),
  `mark_running` (commits `running` to `main` *before* the worktree is cut),
  `finalize_agent_run`/`mark_completed` (marks the agent `completed` only on a
  zero exit; non-zero exits stay `running`).
- `src/agent/stop.rs` — `stop_agents_with_systems` dispatches the per-system
  abort: `Opencode => abort_opencode_session`, `Claude => kill_claude_session`.
- `src/process.rs` — `run_forwarding`, the shared "inherit stdio + run
  `on_started`" primitive both attached runners build on.

The claude path is the closest analogue for codex (a local CLI launched in the
worktree, no HTTP server), so the codex path mirrors claude throughout.

## Findings from the codex source

Studied at `/home/cvoegele/code/github.com/openai/codex` (Rust workspace
`codex-rs`):

- **Entrypoint:** `codex exec [OPTIONS] [PROMPT]`
  (`codex-rs/exec/src/cli.rs`). The prompt is the positional `[PROMPT]`; if
  omitted (or `-`), it is read from stdin.
- **Flags relevant to waap** (`codex-rs/exec/src/cli.rs` +
  `codex-rs/utils/cli/src/shared_options.rs`):
  - `--model` / `-m <MODEL>` (global) — selects the model.
  - `--json` (alias `--experimental-json`, global) — emit JSONL events to
    stdout (`thread.started`, `turn.started/completed/failed`, `item.*`,
    `error`).
  - `--output-last-message <FILE>` / `-o` (global) — write the agent's last
    message to `FILE`.
  - `--dangerously-bypass-approvals-and-sandbox` (alias `--yolo`) — skip every
    confirmation prompt and run with no sandbox. This is the codex equivalent of
    claude's `--permission-mode auto` + `"sandbox":{"enabled":false}`.
  - `--sandbox` / `-s <MODE>` — finer-grained sandbox selection (alternative to
    the blanket bypass above).
  - `--skip-git-repo-check` (global) — allow running outside a git repo.
  - `--ephemeral` (global) — do not persist session files to `$CODEX_HOME`.
  - `--cd` / `-C <DIR>` — set the agent's working root (alternative to setting
    the child process `current_dir`).
  - `-c <key>=<value>` config overrides (`CliConfigOverrides`); unknown keys are
    tolerated unless `--strict-config` is set.
  - `codex exec resume <SESSION_ID> | --last [PROMPT]` — resume a prior session.
- **Session id semantics (important).** Codex generates its own id; it is
  **not** pre-assignable the way claude accepts `--session-id <uuid>`. In
  `--json` mode the first event is
  `thread.started { "thread_id": "<uuid>" }`
  (`codex-rs/exec/src/exec_events.rs:39`,
  `codex-rs/exec/src/event_processor_with_jsonl_output.rs:394`). The positional
  `SESSION_ID` accepted by `codex exec resume` is resolved as a thread id when
  it parses as a UUID (`resolve_resume_thread_id`,
  `codex-rs/exec/src/lib.rs:1450`), so **`thread_id` is the value to capture if
  authentic resume is ever needed.** Critically, `thread_id` never appears on
  the codex process argv — codex prints it, it is not passed in.
- **Exit codes.** `codex exec` exits `0` on success and `1` when any error or
  `turn.failed` event is seen (`error_seen` ⇒ `std::process::exit(1)`,
  `codex-rs/exec/src/lib.rs:1047`). This matches the contract
  `finalize_agent_run` already relies on: a zero exit ⇒ mark `completed`, a
  non-zero exit ⇒ leave `running`.

---

## 1. `AgentSystem::Codex` variant and CLI wiring

In `src/agent.rs`:

- Add `Codex` to the `AgentSystem` enum.
- `as_str`: `AgentSystem::Codex => "codex"`.
- `parse` and `labels` need no change — both iterate
  `AgentSystem::value_variants()`, so the new variant is picked up
  automatically. `labels()` (used by `require_optional_string_choice` when
  validating the `system` frontmatter field) will now accept `"codex"`.

In `src/cli.rs`:

- No structural change — `--system` is `#[arg(long, value_enum)]` over
  `AgentSystem`, so clap accepts `--system codex` once the variant exists.

**Tests that change:**

- `src/cli.rs::agent_run_rejects_invalid_system_argument` currently asserts that
  `--system codex` is an `InvalidValue` error. **This test must be replaced** by
  a positive test (e.g. `parses_agent_run_system_codex`) asserting it parses to
  `AgentSystem::Codex`, plus a new negative test using a value that is still
  invalid (e.g. `--system gemini`).
- `src/cli.rs::parses_agent_run_system_argument` (claude) stays; add the codex
  analogue beside it.
- Optionally add an `agent.rs` round-trip assertion that
  `AgentSystem::parse("codex") == Some(AgentSystem::Codex)` and
  `Codex.as_str() == "codex"`.

## 2. Building the `codex exec` command

Add `src/codex.rs` mirroring `src/claude.rs`:

```rust
pub(crate) struct CodexRunConfig {
    pub(crate) model: Option<String>,
    pub(crate) repo_root: PathBuf,
}

pub(crate) struct CodexRunCommand {
    pub(crate) program: String,     // "codex"
    pub(crate) args: Vec<String>,
    pub(crate) working_dir: PathBuf, // the prepared worktree
}
```

`build_codex_run_command(config, agent_id, session_id) -> CodexRunCommand`
produces:

```
codex exec \
  --json \
  --dangerously-bypass-approvals-and-sandbox \
  --skip-git-repo-check \
  --output-last-message <working_dir>/.waap-codex-last-message-<session_id>.txt \
  [--model <CODEX_MODEL>] \
  "Complete when instructions in /.waap/agents/<agent-id>/agent.md are satisfied"
```

Rationale for each flag:

- **`exec`** — the non-interactive subcommand (first positional arg after the
  program name).
- **prompt** — byte-for-byte the same sentence claude/opencode use:
  `format!("Complete when instructions in /.waap/agents/{agent_id}/agent.md are satisfied")`,
  the last element of `args`.
- **`--json`** — consistent with claude's `--output-format json` and opencode's
  `--format json`. waap does not parse it (status comes from the exit code), but
  it keeps machine-readable output and is required if a later iteration wants to
  parse `thread.started`. (If human-friendly terminal output is preferred over
  consistency, `--json` may be dropped with no behavioral impact.)
- **`--dangerously-bypass-approvals-and-sandbox`** — the codex analogue of
  claude's `--permission-mode auto` + disabled bash sandbox; required for an
  unattended agent to run shell/edit/git commands (including the agent's own
  `git rebase`/`--ff-only` merge) without prompting. waap already runs each
  agent in an isolated worktree, so the external-sandbox precondition this flag
  documents is satisfied.
- **`--skip-git-repo-check`** — defensive. The worktree *is* a valid git
  worktree so the check would pass anyway, but including it removes any
  dependence on codex's repo-detection heuristics.
- **`--output-last-message <file>`** — see §4: the file path embeds
  `session_id`, which is how waap places a unique, collision-free token on the
  codex argv for the stop path; capturing the final message is a useful
  byproduct. The file lives inside the worktree and is discarded with it.
- **`--model`** — appended only when `CODEX_MODEL` is set (see §7), mirroring
  claude's optional `--model`.

Working directory: set `working_dir = worktree` and launch with
`Command::current_dir(&working_dir)` exactly as claude does. (`--cd <worktree>`
is an equivalent alternative; prefer `current_dir` for parity with claude.)
**Do not** add `--ephemeral`: keeping the default session persistence under
`$CODEX_HOME` aids debugging and is the prerequisite for any future resume
support; it is otherwise harmless.

`codex_run_config_from_env(repo_root)` mirrors `claude_run_config_from_env`:
reads `CODEX_MODEL`, canonicalizes `repo_root`.

## 3. Attached run (forwarding + `on_started` + exit code)

Add `run_codex_attached(command, on_started)` identical in shape to
`run_claude_attached` (`src/claude.rs:47`): build a `std::process::Command` from
`command.program`/`args`, set `current_dir(&command.working_dir)`, and delegate
to `crate::process::run_forwarding(&mut process, on_started)`. This reuses the
shared forwarding primitive unchanged, so:

- stdout/stderr inherit and forward to waap's stdout/stderr;
- `on_started` fires once the child is spawned (used today only as `|| Ok(())`);
- the child's `ExitStatus` is returned and flows back through
  `finalize_agent_run`, which marks the agent `completed` on a zero exit and
  leaves it `running` otherwise.

In `src/agent/run.rs` add `run_agent_codex`, modeled on `run_agent_claude`
(`src/agent/run.rs:111`):

```rust
fn run_agent_codex(repo_root, output_format, agent_id) -> io::Result<ExitCode> {
    let mut config = codex_run_config_from_env(repo_root)?;
    let session_id = Uuid::new_v4().to_string();           // see §4

    let (mut metadata, body) = read_agent_record(repo_root, agent_id)?;
    metadata.session_id = Some(session_id.clone());
    metadata.system = Some(AgentSystem::Codex);

    let status = run_in_agent_worktree(
        repo_root, agent_id,
        || mark_running(repo_root, output_format, agent_id, &mut metadata, &body),
        |worktree| {
            config.repo_root = worktree.to_path_buf();
            let command = build_codex_run_command(&config, agent_id, &session_id);
            run_codex_attached(&command, || Ok(()))
        },
    )?;
    finalize_agent_run(repo_root, output_format, agent_id, status)
}
```

Extend the `run_agent` dispatch (`src/agent/run.rs:50`) with
`AgentSystem::Codex => run_agent_codex(repo_root, output_format, agent_id)`.
The worktree lifecycle (`run_in_agent_worktree`, `mark_running`,
`finalize_agent_run`) is reused verbatim — no changes there.

## 4. Session-id strategy (the key open question) — RESOLUTION

**The constraint.** Codex generates its own session/thread id and does *not*
accept a caller-supplied one (no `--session-id` equivalent). The authentic id
(`thread_id`) is only observable two ways: (a) by parsing the `--json`
`thread.started` event off stdout, or (b) by inspecting freshly written session
files under `$CODEX_HOME`. Furthermore, `thread_id` never appears on the codex
process argv, so it cannot be used as a `pkill -f` pattern.

**Two distinct needs pull in different directions:**

1. *Metadata / traceability / future resume* wants codex's **authentic
   `thread_id`**.
2. *`waap agent stop`* (which mirrors claude's `pkill -TERM -f <session_id>`)
   needs a token that is **unique to the codex child and present on its argv,
   and absent from the parent `waap agent run` process** (otherwise the stop
   kills its own parent).

   - The agent id is *not* usable: it is present on the codex prompt **and** on
     the parent `waap agent run --agent-id <id> --system codex` argv, so
     `pkill -f <agent-id>` would also kill the running waap parent.
   - `thread_id` is *not* usable: it is unique but never on the argv.

**Options considered:**

- **(A) Parse `thread.started.thread_id` from `--json` stdout.** Gives the
  authentic id (enables `codex exec resume <thread_id>` and accurate
  traceability). Costs: waap must capture/scan stdout instead of letting it
  inherit, which breaks the simple `run_forwarding` inherit-stdio model
  (`src/process.rs`) that §3 reuses — stdout would need to be piped, scanned for
  the first event, and re-emitted. And it still does **not** solve stop, because
  `thread_id` is not on the argv.
- **(B) Scrape `$CODEX_HOME/sessions` for the newest session file.** Authentic
  id without touching stdout, but racy (concurrent agents), filesystem-layout
  dependent, and broken by `--ephemeral`. Rejected as fragile.
- **(C) Synthesize a waap UUID up front (mirror claude exactly) and place it on
  the codex argv via the `--output-last-message` file path.** `session_id =
  Uuid::new_v4()`, passed as
  `--output-last-message <worktree>/.waap-codex-last-message-<session_id>.txt`.
  The UUID is now on the codex child's argv (and absent from the parent's), so
  `pkill -TERM -f <session_id>` targets exactly the codex child — byte-identical
  to `kill_claude_session`. No stdout capture, no `run_forwarding` change, no
  schema change. Cost: `session_id` is a waap correlation id, **not** codex's
  authentic `thread_id`, so waap cannot itself drive `codex exec resume` from
  it.

**Recommendation: Option (C).** It mirrors the claude lifecycle with the highest
fidelity, requires no change to the shared forwarding primitive, keeps the
`AgentMetadata` schema untouched, and fully satisfies the two things waap
actually uses `session_id` for today — frontmatter/traceability and
`waap agent stop`. waap does not currently invoke `codex exec resume`, so
sacrificing waap-driven resume is acceptable. The marker is carried by a
genuinely useful flag (`--output-last-message`), not an inert hack.

**Documented tradeoff / upgrade path.** If authentic-id resume becomes a
requirement, switch to a hybrid: adopt Option (A) to record the real
`thread_id` (in `session_id` or a new field) for resume/traceability, and keep
an Option (C)-style argv marker purely for the stop pattern. That is a strictly
larger change (stdout capture + possibly a schema field) and is intentionally
out of scope here.

## 5. `waap agent stop` abort path for codex

Add `kill_codex_session(session_id)` to `src/codex.rs`, identical to
`kill_claude_session` (`src/claude.rs:30`):

```rust
pub(crate) fn kill_codex_session(session_id: &str) -> io::Result<()> {
    // pkill -TERM -f <session_id>; treat exit 0 (signalled) and 1 (no match) as success.
    ...
}
```

In `src/agent/stop.rs::stop_agents_with_systems` (`src/agent/stop.rs:46`) add the
dispatch arm:

```rust
AgentSystem::Codex => kill_codex_session(session_id),
```

This is consistent with the Option (C) session-id strategy: the `session_id`
stored in the agent record is exactly the UUID embedded in the codex argv
(via `--output-last-message`), so `pkill -f <session_id>` matches the codex
child and nothing else. No change to `stop_agents`/`stop_agent_if_running` or to
the `abort(system, session_id)` closure signature is required.

## 6. Worktree integration

`run_in_agent_worktree` already cuts `worktrees/<agent-id>` from the
`running`-status commit, runs the system inside it, and removes it afterward
(even on error/non-zero exit). Codex reuses this unchanged:

- Launch codex with `current_dir = worktree` (§3).
- The worktree is a valid git worktree, so codex's git-repo detection succeeds;
  `--skip-git-repo-check` is included defensively regardless (§2).
- `--output-last-message` writes inside the worktree and is removed with it.
- `CODEX_HOME` is **not** the worktree — it is codex's config/auth/session home
  (default `~/.codex`), inherited from the environment like claude's auth. waap
  does not set or relocate it. With default (non-`--ephemeral`) behavior, session
  files accumulate under `$CODEX_HOME/sessions`; this is outside the worktree and
  is not waap's concern.

## 7. Config / env

- **Model:** add `CODEX_MODEL`, mirroring `CLAUDE_MODEL`.
  `codex_run_config_from_env` reads it with
  `env::var("CODEX_MODEL").ok().filter(|m| !m.is_empty())`; when set,
  `build_codex_run_command` appends `--model <value>`, otherwise codex uses its
  configured default.
- **`CODEX_HOME` / auth:** assumed pre-configured in the environment (API key or
  prior `codex login`), exactly as claude assumes its own auth is present. waap
  reads no codex auth env vars and sets no `CODEX_HOME`. Document this as an
  operator precondition for `--system codex` (parallel to the existing
  `OPENCODE_SERVER_*` and claude-auth assumptions). Unlike opencode,
  `codex_run_config_from_env` has **no required env vars** — `CODEX_MODEL` is
  optional — so it never fails for missing configuration.

## 8. Test plan (mirror the claude/opencode unit tests)

In `src/codex.rs` (mirroring `src/claude.rs` tests):

- `codex_run_command_matches_spec` — assert exact `program`/`args`/`working_dir`
  for a config with `CODEX_MODEL` set, including `exec`, `--json`,
  `--dangerously-bypass-approvals-and-sandbox`, `--skip-git-repo-check`, the
  `--output-last-message` path containing the session id, `--model <model>`, and
  the trailing prompt string.
- `codex_run_command_omits_model_when_unset` — no `--model` when model is `None`;
  last arg is the prompt.
- `run_codex_attached_propagates_exit_code_and_marks_started` — run
  `sh -c "exit 5"`, assert `status.code() == Some(5)` and `on_started` ran
  (mirror `run_claude_attached_propagates_exit_code_and_marks_started`).
- (Optional) a test asserting the `--output-last-message` filename contains the
  session id, since the stop path depends on that token being on the argv.

In `src/cli.rs`:

- Replace `agent_run_rejects_invalid_system_argument` (currently expects `codex`
  to be invalid) with `parses_agent_run_system_codex` (asserts
  `AgentSystem::Codex`) and a new negative test using a still-invalid value.

In `src/agent.rs`:

- Extend the metadata round-trip test(s) so `system = "codex"` parses; assert
  `AgentSystem::parse("codex")`/`as_str()` round-trip.

In `src/agent/stop.rs`:

- Add `agent_stop_kills_codex_process` modeled on
  `agent_stop_kills_claude_process_instead_of_opencode_abort`
  (`src/agent/stop.rs:262`): seed a running agent with `system = "codex"` and a
  session id, stop it via the injected closure, assert the `Codex` arm fired
  with the session id (not the opencode/claude arms) and the record becomes
  `aborted`.

`finalize_agent_run` exit-code tests already cover the completed/running
transitions and are system-agnostic, so they need no codex-specific additions.

## 9. Open questions / risks for human review

1. **Authentic session id vs. resume.** Option (C) stores a waap UUID, not
   codex's `thread_id`, so waap cannot drive `codex exec resume`. Confirm
   waap-driven resume is genuinely out of scope; if not, adopt the §4 hybrid
   (stdout parsing + schema field).
2. **`--dangerously-bypass-approvals-and-sandbox` blast radius.** This disables
   codex's sandbox entirely. It is justified because each agent runs in an
   isolated worktree, but reviewers should confirm that matches the threat model
   accepted for claude's disabled bash sandbox. A narrower `--sandbox
   workspace-write` is the alternative if full bypass is unacceptable.
   **DECIDED (operator review): use full bypass to match the claude path.**
3. **`pkill -f` matching breadth.** The stop pattern is the session UUID, which
   is collision-free against the waap parent and other agents. Confirm no other
   tooling writes that UUID onto an unrelated process's argv. (Same residual
   risk class as the existing `kill_claude_session`.)
4. **`--json` terminal readability.** With `--json`, the attached run streams
   JSONL to the operator's terminal rather than codex's prettier human output.
   Decide whether consistency with claude/opencode (keep `--json`) or
   readability (drop it) wins; behavior is identical either way.
   **DECIDED (operator review): keep `--json` for consistency.**
5. **codex CLI version drift.** Flag names/aliases
   (`--json`/`--experimental-json`, `--dangerously-bypass-approvals-and-sandbox`/
   `--yolo`) and the `thread.started` JSONL shape are taken from the pinned
   source tree. Pin or document a minimum supported `codex` version, since these
   are still evolving in the codex repo.
6. **Auth/`CODEX_HOME` provisioning.** The deployment must ensure codex auth is
   present in the run environment. Unlike opencode, waap performs no startup
   validation of codex config, so a misconfigured environment surfaces only as a
   codex runtime error (non-zero exit ⇒ agent left `running`).
