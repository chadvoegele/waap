+++
name = "Add Codex model and reasoning-effort run options"
creation_date = 2026-08-03T17:04:24Z
status = "completed"
+++

# Problem

`waap agent run --system codex` already reads optional `CODEX_MODEL` and sends it as `model` in Codex app-server `thread/start` and `turn/start` requests. However, the command has no per-run model option and cannot configure Codex reasoning effort. OpenCode supports a model plus thinking variant through `OPENCODE_SERVER_MODEL=provider/model[/variant]`, but Codex uses a distinct app-server field: `turn/start.effort`.

Users need explicit, per-run Codex model and reasoning controls without removing existing environment-based automation.

# Required behavior

Add Codex-only options to `waap agent run`:

```text
waap agent run --agent-id <id> --system codex \
  --model <MODEL> \
  --reasoning-effort <EFFORT>
```

- `--model` is optional and overrides `CODEX_MODEL` for this run.
- `--reasoning-effort` is optional and overrides a new optional `CODEX_REASONING_EFFORT` environment variable.
- Preserve current behavior when neither CLI option nor its environment fallback is set: Codex uses its configured defaults.
- Reject either option when `--system` is `opencode` or `claude`; do not silently ignore it or change those backends.
- Reject empty model CLI values.
- Accept exactly these reasoning efforts from both CLI and environment configuration: `none`, `minimal`, `low`, `medium`, `high`, `xhigh`, `max`, and `ultra`. Reject invalid environment values with an `io::ErrorKind::InvalidInput` error that names `CODEX_REASONING_EFFORT` and lists the accepted values. Clap should report invalid CLI values before command dispatch.
- Resolve and validate run options before mutating agent state. Invalid configuration must not mark an agent running or create a worktree.

# Codex protocol behavior

The installed Codex 0.144.5 app-server schema and current Codex protocol use:

- `thread/start.model` for the initial model;
- `turn/start.model` to override the model for the turn and subsequent turns;
- `turn/start.effort` to override reasoning effort for the turn and subsequent turns.

Continue sending the resolved model in both `thread/start` and `turn/start`, preserving current behavior. Send resolved reasoning effort only as the top-level `effort` field in `turn/start`; omit it when unset. Do not send `reasoningEffort` or add effort to `thread/start`, because the supported `ThreadStartParams` does not expose it.

# Implementation guidance

Likely files:

- `src/cli.rs`: add the two optional run arguments and parser tests.
- `src/app.rs`: pass run options into agent execution.
- `src/agent.rs` and/or `src/agent/run.rs`: represent run overrides, enforce their Codex-only scope, and pass them into backend construction without weakening lazy backend configuration.
- `src/agent/codex.rs`: resolve CLI-over-environment precedence, validate environment effort, retain the resolved effort in `CodexRunConfig`/`CodexClient`, and encode it in `turn_start_params`.
- `specs/spec.md`, `specs/codex-agent-system.md`, and `.agents/skills/waap/SKILL.md`: document CLI options, environment fallbacks, precedence, accepted efforts, and Codex-only scope.

Prefer a typed `CodexReasoningEffort`/Clap value enum or an equivalent single source of truth so CLI parsing, environment validation, protocol serialization, help output, and tests cannot drift. Keep `AgentSystem::backend` configuration lazy: selecting Claude or Codex must not read OpenCode credentials, and selecting a non-Codex backend must not require Codex environment variables.

# Acceptance criteria

1. `--system codex --model gpt-5.4 --reasoning-effort high` sends `model: "gpt-5.4"` in both `thread/start` and `turn/start`, plus `effort: "high"` in `turn/start` only.
2. CLI values take precedence over conflicting `CODEX_MODEL` and `CODEX_REASONING_EFFORT` values.
3. Without CLI values, `CODEX_MODEL` and `CODEX_REASONING_EFFORT` are used when present.
4. When neither source is set, all model/effort fields continue to be omitted as today.
5. Invalid CLI effort is rejected by Clap. Invalid `CODEX_REASONING_EFFORT` is rejected before agent state changes. Empty/invalid model input is rejected clearly.
6. Passing either new option with OpenCode or Claude returns a clear error before agent state changes.
7. Existing OpenCode provider/model/variant behavior and Claude model behavior remain unchanged.
8. Documentation includes an example command and the precedence rules.

# Tests and validation

Add focused tests for:

- CLI parsing with both options, each option independently, and invalid effort;
- Codex-only option rejection for OpenCode and Claude without backend startup or state mutation;
- CLI-over-environment precedence and environment-only fallback;
- every accepted effort plus invalid/empty environment values;
- `thread_start_params` preserving model behavior and never including effort;
- `turn_start_params` including/omitting model and effort in all relevant combinations;
- unchanged backend environment isolation.

Run from the repository root:

```sh
cargo clippy --all-targets -- -D warnings
cargo fmt --check
cargo build
cargo build --release
cargo test
cargo run -- check
waap check
```

Per repository instructions, run `cargo test` outside any command sandbox.
