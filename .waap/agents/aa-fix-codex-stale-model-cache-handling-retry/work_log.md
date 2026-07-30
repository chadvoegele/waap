# Work log

Ticket: `tt-handle-incompatible-codex-model-cache-schema`

- Marked the ticket in progress and inspected WAAP's Codex app-server client and
  Codex's model-cache implementation.
- Installed Codex CLI 0.144.5 in `/tmp/waap-codex-repro.JSPYvg` and used an
  isolated `CODEX_HOME` containing copies of auth, config, and model-cache files.
  The original Codex state was not modified.
- Exercised the real `initialize`, `model/list`, `thread/start`, and `turn/start`
  request flow. A stale cache present at startup was refreshed before use, and
  the turn completed. Replacing the isolated cache with the stale schema after
  `thread/start` also completed without a warning in that run.
- Root cause: Codex logs the warning from
  `OpenAiModelsManager::refresh_if_new_etag` when an unchanged response ETag
  causes `renew_cache_ttl` to deserialize a cache rewritten with an incompatible
  schema. The renewal error is logged and discarded, so it does not fail the
  request. The observed shared cache had `client_version = "0.144.5"` and lacked
  `supports_reasoning_summaries`, consistent with concurrent stale-schema
  rewriting.
- Changed only Codex app-server stderr handling. WAAP now pipes and line-forwards
  stderr, filtering the exact model-manager/TTL/missing-field combination.
  Other Codex diagnostics, including other cache-renewal errors, remain intact.
- Added regression tests for unchanged normal stderr and targeted filtering with
  surrounding unrelated errors.
- Required validations passed:
  `cargo clippy --all-targets -- -D warnings`, `cargo fmt --check`,
  `cargo build`, `cargo build --release`, `cargo test` (298 total unit and
  integration tests), and `waap check`.
- Rebased onto current `origin/main`, pushed
  `aa-fix-codex-stale-model-cache-handling-retry`, and opened
  https://github.com/chadvoegele/waap/pull/1 against `main`.
