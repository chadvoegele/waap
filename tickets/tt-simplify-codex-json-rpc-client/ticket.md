+++
name = "Simplify Codex JSON-RPC client"
creation_date = 2026-07-10T02:43:53Z
status = "completed"
+++

# Problem

`src/agent/codex.rs` contains unnecessary state, one-use helpers, defensive protocol handling, and extensive comments that make the small JSON-RPC client harder to scan than its behavior warrants.

In particular, `CodexClient` stores an unread `Option<Child>` under `#[allow(dead_code)]`. A `Child` is not an RAII process-lifetime guard: dropping it does not terminate or wait for the process. The retained stdin/stdout pipes are what keep the connection usable, and closing stdin causes the app-server to exit.

# Desired Refactor

Simplify `src/agent/codex.rs` while keeping the JSON-RPC flow explicit and avoiding new abstractions that merely move complexity:

- Remove `child: Option<Child>`, its lint allowance, and the `Child` import.
- Remove the test-only `CodexClient::new` constructor that exists to populate `child`; construct test clients directly.
- Specialize `notification_line` for its only use: notifications without params. Serialize `{ "method": method }` directly instead of building a `serde_json::Map` and accepting `Option<JsonValue>`.
- Remove `response_id` and inline response-id correlation in `send_request` with a straightforward early `continue`.
- Remove `turn_interrupt_params` and inline its small JSON object in `turn_interrupt`.
- Make `TurnStatus::from_wire` accept only the canonical protocol values: `completed`, `interrupted`, `failed`, and `inProgress`. Remove the speculative PascalCase aliases.
- Remove tests made redundant by inlining, while retaining behavioral coverage through the client-level tests.

Do not add replacement helpers unless they clearly reduce the total cognitive load. Keep the generic reader, writer, and output transports, the separate thread/turn parameter builders, `interrupt_sent`, and `TurnStatus::is_success`.

# Comment Cleanup

Remove essentially all comments and doc comments from `src/agent/codex.rs`, including comments in tests. The code, function names, constants, and tests should explain the implementation.

The finished file should contain no more than 0–2 total `//` or `///` comment lines. Keep a comment only if it explains a critical protocol or lifecycle fact that cannot be made clear through code structure or naming. Do not retain section introductions, source references, comments that restate control flow, test narration, or historical rationale.

# Acceptance Criteria

- `CodexClient` no longer stores `Child` and no `#[allow(dead_code)]` remains.
- Production behavior still relies on the retained stdio pipes and app-server EOF behavior; no process-management abstraction is added.
- `notification_line` has no unused params generality.
- `response_id` and `turn_interrupt_params` are removed.
- Only canonical Codex turn-status spellings are accepted.
- Existing request framing, response correlation, error propagation, delta forwarding, initialization, thread/turn startup, interruption, and completion behavior remain covered.
- `src/agent/codex.rs` contains at most two total comment/doc-comment lines.
- No unrelated files are changed except as required by formatting or compilation.
- Run all repository validations from the repository root:

```sh
cargo clippy --all-targets -- -D warnings
cargo fmt --check
cargo build
cargo build --release
cargo test
```
