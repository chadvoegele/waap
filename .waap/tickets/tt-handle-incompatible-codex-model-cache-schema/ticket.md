+++
name = "Handle incompatible Codex model cache schema"
creation_date = 2026-07-30T15:25:23Z
status = "in-progress"
+++

## Problem

Running a WAAP agent with `--system codex` emitted this Codex CLI 0.144.5 error while starting the app server:

```text
ERROR codex_models_manager::manager: failed to renew cache TTL: missing field `supports_reasoning_summaries` at line 87 column 5
```

The shared `~/.codex/models_cache.json` was rewritten shortly afterward and a direct app-server initialization then succeeded. Attempts to recreate the warning by removing `supports_reasoning_summaries` from a copied cache did not reproduce it consistently. This suggests a transient stale/incompatible Codex cache, possibly during a CLI schema upgrade, but WAAP currently inherits Codex stderr directly and offers no diagnosis or recovery.

## Work

1. Reproduce the failure deterministically if possible, including the actual WAAP Codex app-server flow (`initialize`, `thread/start`, and `turn/start`).
2. Identify whether WAAP can safely prevent or recover from the incompatible model-cache state without deleting user configuration, credentials, sessions, or unrelated caches.
3. Implement the smallest robust WAAP-side fix. Do not merely suppress all Codex stderr or hide unrelated errors. If the upstream error is harmless, ensure the fix targets only this known stale-cache condition and preserves useful diagnostics.
4. Add regression tests that fail before the fix and cover normal Codex startup plus the stale-cache/error path.
5. Document any behavior or compatibility assumptions that users need to know.

## Acceptance criteria

- `waap agent run --system codex` no longer exposes or fails because of the known missing-`supports_reasoning_summaries` stale-cache condition, or performs a safe targeted recovery and retry when it prevents startup.
- Other Codex stderr and startup failures remain visible and actionable.
- Existing Codex request/notification behavior remains unchanged.
- `waap check` passes.
- Run all repository validations from `AGENTS.md`: `cargo clippy --all-targets -- -D warnings`, `cargo fmt --check`, `cargo build`, `cargo build --release`, and `cargo test` (outside the command sandbox).
- Commit the implementation on the agent feature branch, rebase it onto current `origin/main`, push the branch, and open a GitHub pull request against `main`. Do not merge the PR.
