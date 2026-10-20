---
name: waap-heat-equation-e2e-test
description: Use when end-to-end testing waap by planning, implementing, and verifying a 2D heat equation finite-difference program.
---

# Heat Equation E2E

This skill describes an end-to-end waap test by making a simple 2D heat equation simulator.

## Steps

1. Create a new repository in a temporary directory.
2. Write a spec for a 2D heat equation simulator using a finite difference method.
3. Use the waap skill to create a ticket to write the implementation tickets based on the spec.
4. Run a waap planner agent for that ticket using Codex, and steer it while active as described below.
5. Repeatedly run waap developer agents for unblocked pending tickets.
6. Run the final program.
7. Verify the output is numerically plausible and satisfies the spec.

## Runtime Prerequisites

Use a `waap` binary on `PATH`. When testing an uninstalled local checkout, prepend the checkout's target directory before running commands in the temporary repository:

```sh
export PATH="/path/to/waap/target/debug:$PATH"
```

Use an authenticated Codex app server with native `turn/steer` support. Set a private runtime directory with a short path for the control socket:

```sh
export XDG_RUNTIME_DIR=$(mktemp -d /tmp/waap-runtime.XXXXXX)
```

Remove this test-created directory only after all runs have exited.

## Test Repository Setup

Create all test files under a temporary directory.

```sh
tmpdir=$(mktemp -d)
cd "$tmpdir"
git init
mkdir -p specs
```

Write `specs/spec.md` with these requirements:

```markdown
# 2D Heat Equation Simulator

Build a small command-line program that simulates the 2D heat equation on a square grid using an explicit finite difference method.

## Requirements

1. The program must be runnable.
2. Use a square grid of at least 20 by 20 cells.
3. Initialize the grid with a hot spot on one edge, rest cool.
4. Advance the system until stable.
5. Use a stable explicit finite difference update for the 2D heat equation.
6. Print or write a final summary containing at least the center temperature, edge temperatures, minimum temperature, maximum temperature, and average temperature.
7. Include a test or verification command that checks the simulation result is plausible.
8. Include a visualization mode that writes an ASCII-art style grid in the terminal.

## Acceptance Criteria

1. The program runs without errors.
2. The final center temperature is higher than its initial temperature.
3. The minimum temperature is non-negative.
4. The maximum temperature does not exceed the initial hot spot temperature.
5. The verification command exits successfully.
```

## Steer the Planner

Start the planner with `waap agent run --agent-id "$agentid" --system codex` in a managed background process or another terminal. From the same temporary repository, wait for its first response or tool event so its turn is active. Keep the initial instructions free of the marker below.

Generate a marker after the run starts, then steer the planner once:

```sh
marker="STEERING_$(python3 -c 'import uuid; print(uuid.uuid4().hex)')"
waap --output-format json agent steer --agent-id "$agentid" \
  --prompt "Before finishing, write exactly $marker to steering-check.txt in your worktree and commit it with your work. Continue planning the heat equation implementation."
```

Require exit code 0 and `accepted: true`. Acceptance alone is not proof of compliance: after the planner completes and integrates its changes, verify:

```sh
test "$(cat steering-check.txt)" = "$marker"
```

Check that steering did not change the agent's session ID, status, or instructions; normal completion may change its status afterward. Verify the planner completes its original ticket and the run removes its worktree and control socket. A later steering call must reject the completed agent.

Record the command results and marker evidence. If the turn ends before submission or acceptance is unknown, report the failure; do not automatically retry or claim steering passed.

## Success Criteria

The end-to-end run succeeds only when:

1. `waap check` passes.
2. All waap tickets and agents are marked as completed.
3. The generated heat equation program runs.
4. The verification command passes.
5. Steering is accepted, its marker is verified, session continuity and cleanup are checked, and steering after completion is rejected.
6. The result is documented with the temporary repository path and commands used.

If any step fails, fix the workflow or generated project and rerun from the failed step until the program is verified.
