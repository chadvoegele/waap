# waap

`waap` is an agent automation platform that structures execution of disposable AI agents.

`waap` is similar in spirit to [Gas Town](https://github.com/gastownhall/gastown), but without the exhaust, and [Omnigent Polly](https://github.com/omnigent-ai/omnigent), but without the omni.

<img width="512" height="512" alt="image" src="https://github.com/user-attachments/assets/929a5b2c-6c08-457a-b16f-d00c34bfa080" />

## Motivation

I found that an effective pattern for running autonomous agents was making a `TODO.md` and pointing the agent to it with a `/goal`. `waap` formalizes this approach, promoting tickets and agents to first class. It persists context into files to get you out of the "my chat history is my context" approach, parallelizes agent execution, and rapidly gets you to [level 8](https://www.augmentcode.com/guides/steve-yegge-8-levels-ai-assisted-development).

waap stores state in a directory under `~/.local/state/waap/data/`. Set `--waap-root <path>` to override the default. `waap check` reports the state directory (`${waap_data}` below).

## Design

`waap` uses two types:

1. `waap` tickets describe implementation work and live at `${waap_data}/tickets/<ticket-id>/ticket.md`.
2. `waap` agents track a registered backend, the latest turn's status, and its session ID once opened. Their metadata lives at `${waap_data}/agents/<agent-id>/agent.md`.

Records use TOML frontmatter for metadata; tickets also have a Markdown body. Prompts are sent directly to the agent, not stored in its record. Run `waap check` to validate the state. Git tracking is optional: if you track state, stage and commit changes yourself.

Frequently but not always, agents are assigned to work on tickets.

Tickets can include one or more dependencies to create arbitrary directed acyclic graphs (DAGs). This allows you to create a multi-step implementation plan up front, specify complex workflows, and also parallelize agent execution as wide as your workflow allows.

## `waap` Workflow

In a `git` repo, initialize a new `waap` project:

```sh
waap init
```

Then create a ticket that describes the work to be done:

```
printf "Implement the file picker" | waap ticket new --name="File Picker"
```

This creates a `ticket.md` file at `${waap_data}/tickets/tt-file-picker/ticket.md` like

```
+++
name = "File Picker"
creation_date = 2026-07-06T10:44:50Z
status = "pending"
+++

Implement the file picker
```

Next register an agent to work on the ticket. Pi is the default backend; choose a different backend with `--system` at registration.

```sh
waap agent new --name "File Picker Implementer" --system pi
# Agent ID: aa-file-picker-implementer
```

Registration creates a metadata-only `agent.md` with `status = "ready"` and does not open a session. Use the returned ID to send instructions, either with `--text` or by redirecting a Markdown file into `agent prompt`:

```sh
waap agent prompt --agent-id aa-file-picker-implementer --delivery=start \
  --text='Continue working on `tt-file-picker` until done.'
# Or: waap agent prompt --agent-id aa-file-picker-implementer --delivery=start < task.md
```

From another terminal, steer the active turn or stop it:

```sh
waap agent prompt --agent-id aa-file-picker-implementer --delivery=steer \
  --text='Focus on keyboard navigation.'
waap agent stop --agent-id aa-file-picker-implementer
```

Without `--delivery`, the default `asap` starts a turn if possible or steers the active turn. Use `start` or `steer` explicitly when only one is intended. Steering is supported by Pi and Codex.

After completion or a stop with a saved session, send another `--delivery=start` prompt to continue the same conversation. Exact resume currently requires Pi 1.5.2, the original launch checkout, and the same session store. Other backends reject resume instead of creating a replacement. The backend runs at the root of the checkout you invoke WAAP from, including linked worktrees; WAAP does not manage those worktrees. See [Agent prompt delivery](specs/agent-turns.md).

## `waap` Loop

For more complex workflows, follow the same idea but with multiple tickets and use a `waap` loop:

1. While there are unblocked tickets,
    1. Register agents to complete the work.
    2. Send each agent its task with `agent prompt --delivery=start`.
    3. Add new tickets as needed.

## CLI

For a full CLI description, check:

```
waap --help
waap ticket --help
waap agent --help
```

## Skill Resources

Use `waap` with an agent referencing the `waap` skill:

1. `.agents/skills/waap/SKILL.md` describes the waap workflow and CLI reference.
2. `.agents/skills/waap/roles/planner/agent.md` has a recommended `agent.md` for a planner role.
3. `.agents/skills/waap/roles/developer/agent.md` has a recommended `agent.md` for a developer role.

The skill also includes examples of using a `waap` loop to build a [software factory](https://github.com/chadvoegele/waap/blob/main/.agents/skills/waap/SKILL.md#software-factory-loop) and to run a [structured agent program](https://github.com/chadvoegele/waap/blob/main/.agents/skills/waap/SKILL.md#structured-agent-program-loop).

## Build

To build:

```sh
cargo build
cargo build --release
```

To run checks:

```sh
cargo clippy --all-targets -- -D warnings
cargo fmt --check
cargo test
python3 -m unittest discover -s tests -p 'test_*.py'
```

To test against the installed Pi 1.5.2 binary and a local model provider stub (no API credentials needed):

```sh
python3 tests/pi_real_smoke.py target/debug/waap --shutdown-notification
```

To run the heat-equation end-to-end test:

Ask an agent to run `.agents/skills/waap-heat-equation-e2e-test/SKILL.md`.
