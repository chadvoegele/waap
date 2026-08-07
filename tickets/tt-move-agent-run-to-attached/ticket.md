+++
title = "Move agent run to attached mode"
creation_date = 2026-06-27T02:21:55Z
status = "completed"
+++

# Problem

Currently `waap agent run` launches an underlying agent system, `opencode` or `claude`, and then detaches and lets the system run in the background.

This causes a few problems:

1. It's hard for the caller to know the agent finished.
2. Any errors from the system are lost.
3. We don't see whether the agent was successful.

# Desired Behavior

`waap agent run` should run the selected system in the foreground, forwarding the system process stdout and stderr to the command's stdout and stderr.

When the selected system exits, `waap agent run` should exit with the same exit code. If the system cannot be launched, `waap agent run` should return a non-zero CLI error.

Users should be able to run agents in the background with standard process tools such as `nohup` or `setsid`. Background behavior should rely on normal shell/process redirection rather than a waap-specific detached mode that discards output.

`waap agent run` should continue to mark the agent status as 'running' once the system has started.

# Acceptance Criteria

1. `waap agent run --system opencode` forwards the OpenCode process stdout to stdout and stderr to stderr.
2. `waap agent run --system claude` forwards the Claude process stdout to stdout and stderr to stderr.
3. `waap agent run` exits with the same exit code as the selected system process for both `opencode` and `claude`.
4. `waap agent run` can be run in the background using standard tools such as `nohup` or `setsid`, with output controlled by normal shell redirection.
5. The current detached mode is gone.
6. Tests cover stdout forwarding, stderr forwarding, exit-code propagation, and background-compatible process setup for both supported systems.
