# waap

`waap` is an agent automation platform that structures execution of disposable AI agents.

`waap` is similar in spirit to [Gas Town](https://github.com/gastownhall/gastown), but without the exhaust, and [Omnigent Polly](https://github.com/omnigent-ai/omnigent), but without the omni.

<img width="512" height="512" alt="image" src="https://github.com/user-attachments/assets/929a5b2c-6c08-457a-b16f-d00c34bfa080" />

## Motivation

I found that an effective pattern for running autonomous agents was making a `TODO.md` and pointing the agent to it with a `/goal`. `waap` formalizes this approach, promoting tickets and agents to first class. It persists context into files to get you out of the "my chat history is my context" approach, parallelizes agent execution, and rapidly gets you to [level 8](https://www.augmentcode.com/guides/steve-yegge-8-levels-ai-assisted-development).

For each repository, waap stores state in a directory under `~/.local/state/waap/data/` followed by the absolute path of the primary checkout, without its leading slash. `waap check` reports the resolved directory, referred to below as `${waap_data}`. Use `--waap-root <path>` to select a different directory.

## Design

`waap` uses two types:

1. `waap` tickets describe implementation work and live at `${waap_data}/tickets/<ticket-id>/ticket.md`.
2. `waap` agents contain agent instructions, e.g. `AGENTS.md`, and live at `${waap_data}/agents/<agent-id>/agent.md`.

The records use TOML frontmatter for structured metadata and a Markdown body for free-form content. Validate the records with `waap check`. You can leave state unversioned or manage its Git history yourself. waap never stages or commits state changes. The local `.waap-runtime/` directory holds the state lock and is ignored by Git.

Frequently but not always, agents are assigned to work on tickets.

Tickets can include one or more dependencies to create arbitrary directed acyclic graphs (DAGs). This allows you to create a multi-step implementation plan up front, specify complex workflows, and also parallelize agent execution as wide as your workflow allows.

## `waap` Workflow

In a `git` repo, initialize a new `waap` project:

```sh
waap init
```

`waap init` creates missing `agents/` and `tickets/` directories without changing existing records. It also accepts a Git checkout you manage as the state root.

If you move the application repository and use the default state path, stop active agents and other waap commands first. Run `waap check` from the new checkout to find the new path, create its parent directories, then move the old plain state directory there. Verify the move with `waap check`. To keep using the old location instead, pass `--waap-root <old-state-directory>`.

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

Next create an agent to work on the ticket:

```
printf "Continue working on `tt-file-picker` until done" | waap agent new --name="File Picker Implementer"
```

This creates an `agent.md` file at `${waap_data}/agents/aa-file-picker-implementer/agent.md` like

```
+++
name = "File Picker Implementer"
creation_date = 2026-07-06T10:47:39Z
status = "ready"
+++

Continue working on `tt-file-picker` until done
```

Then run your agent with `waap agent run --agent-id aa-file-picker-implementer`.

Steer a running agent with `waap agent steer --agent-id aa-file-picker-implementer --prompt "Focus on keyboard navigation"`.
Stop a running agent with `waap agent stop --agent-id aa-file-picker-implementer`.

## `waap` Loop

For more complex workflows, follow the same idea but with multiple tickets and use a `waap` loop:

1. While there are unblocked tickets,
    1. Create agents to complete the work.
    2. Run the agents.
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
cargo clippy
cargo fmt --check
cargo test
```

To run the end-to-end test:

Ask an agent to run `.agents/skills/waap-heat-equation-e2e-test/SKILL.md`.
