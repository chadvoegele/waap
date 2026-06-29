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
4. Run a waap planner agent for that ticket.
5. Repeatedly run waap developer agents for unblocked pending tickets.
6. Run the final program.
7. Verify the output is numerically plausible and satisfies the spec.

## Runtime Prerequisites

Use a `waap` binary on `PATH`. When testing an uninstalled local checkout, prepend the checkout's target directory before running commands in the temporary repository:

```sh
export PATH="/path/to/waap/target/debug:$PATH"
```

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

## Success Criteria

The end-to-end run succeeds only when:

1. `waap check` passes.
2. All waap tickets and agents are marked as completed.
3. The generated heat equation program runs.
4. The verification command passes.
5. The result is documented with the temporary repository path and commands used.

If any step fails, fix the workflow or generated project and rerun from the failed step until the program is verified.
