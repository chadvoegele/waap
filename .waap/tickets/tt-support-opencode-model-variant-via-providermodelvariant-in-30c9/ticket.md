+++
name = "Support OpenCode model variant via provider/model/variant in OPENCODE_SERVER_MODEL"
creation_date = 2026-07-12T01:01:02Z
status = "pending"
+++

# Problem

`OPENCODE_SERVER_MODEL` is parsed with `split_once('/')` (first slash only) at `src/agent/opencode.rs:118-135`, so only `provider/model` is accepted. OpenCode's HTTP `prompt_async` endpoint accepts an optional top-level `variant` string on the `PromptInput` body (`packages/opencode/src/session/prompt.ts:1594-1615`), which selects the model's thinking/reasoning configuration — e.g. OpenAI `none`/`minimal`/`low`/`medium`/`high`/`xhigh`, Anthropic `high`/`max`, Google `low`/`high`. waap never sends `variant` in `prompt_payload` (`src/agent/opencode.rs:244-253`), so every run falls back to the `build` agent's default variant (currently `undefined`). There is no way to control thinking level per waap run.

# Required Behavior

- Extend `OPENCODE_SERVER_MODEL` to accept the OpenCode ACP convention `provider/model/variant`, matching `parseModelSelection` in opencode's `packages/opencode/src/acp/config-option.ts:115-146`. The trailing segment after the last `/` is the variant when it does not match a known model id under the provider.
- Continue to accept the existing two-segment `provider/model` form with no variant (backwards compatible).
- Forward the parsed `variant` to the OpenCode HTTP API by including `"variant": <value>` in the `POST /session/{sessionID}/prompt_async` body when set; omit the field entirely when no variant is configured so OpenCode falls back to the agent default.
- Keep `OPENCODE_SERVER_URL`, `OPENCODE_SERVER_USERNAME`, `OPENCODE_SERVER_PASSWORD`, and `OPENCODE_SERVER_MODEL` as the only required env vars. No new env var.
- Reject empty/whitespace variant segments and malformed inputs with clear `io::Error` messages consistent with existing `parse_opencode_model` errors.
- Update `specs/spec.md` (the opencode section around line 250) to document the `provider/model/variant` format and that the variant is forwarded to `prompt_async`.
- Update the locked test at `src/agent/opencode.rs:478-490` — `opencode_model_parsing_preserves_nested_model_ids` currently asserts `openai/gpt-5.5/reasoning` parses to `model_id="gpt-5.5/reasoning"`. Either replace with a model id that genuinely contains a slash (if any real model id does) or change the assertion to reflect the new three-segment variant parsing.
- Add tests covering: three-segment parse into `{provider_id, model_id, variant}`, two-segment parse with `variant=None`, payload includes `variant` when set, payload omits `variant` when absent, and invalid forms are rejected.
- Keep the Codex backend unchanged.

# Acceptance Criteria

- `OPENCODE_SERVER_MODEL="openai/gpt-5/high"` produces a `prompt_async` body with `"model": {"providerID":"openai","modelID":"gpt-5"}` and `"variant":"high"`.
- `OPENCODE_SERVER_MODEL="openai/gpt-5"` (no trailing segment) produces a body with no `variant` key, identical to current behavior.
- Real model ids containing slashes (if any) still parse correctly — verify against the OpenCode models.dev catalog or document the assumed constraint.
- `parse_opencode_model` rejects `openai/`, `/gpt-5`, `openai`, and empty variant segments like `openai/gpt-5/`.
- `cargo clippy --all-targets -- -D warnings`, `cargo fmt --check`, `cargo build`, `cargo build --release`, `cargo test`, `cargo run -- check`, and `waap check` pass.
