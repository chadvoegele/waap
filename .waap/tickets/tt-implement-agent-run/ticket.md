+++
title = "Implement Agent Run"
creation_date = 2026-06-23T02:20:09Z
status = "completed"
+++

# Spec Reference
Lines 193-200 and 236-249 of /specs/spec.md

# Description
Implement the `waap agent run --agent-id <agent-id>` CLI behavior enough to launch the agent harness for an existing agent.

# Requirements
- Add the `waap agent run --agent-id <agent-id>` command.
- Validate that the requested agent exists and has a valid `agent.md`.
- Start `opencode run` with the repository root, configured server credentials, and a goal referencing the agent instructions in `.waap/agents/<agent-id>/agent.md`.
- Report the agent path and metadata to stdout in human-readable and JSON output formats.
- Add tests for argument parsing, missing agent handling, command construction, and output shape where practical.

# Todo / Follow-up
- Updating agent metadata to `running` before launch can remain a follow-up if it is not needed for the first runnable implementation.
- Extracting the opencode session id and writing `session_id` back to `agent.md` can remain a follow-up.
