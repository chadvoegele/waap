# Work Log

- Read the agent instructions, ticket, module consumers, adapter tests, and live spec references.
- Marked `tt-move-agent-system-adapters-under-srcagent` in progress.
- Moved the Claude, Codex, and OpenCode adapters under `src/agent/`, updated module paths, and narrowed their visibility to the agent tree.
- Updated the Codex implementation paths in `specs/codex-agent-system.md`.
- Passed clippy, formatting, debug/release builds, and all 234 tests.
