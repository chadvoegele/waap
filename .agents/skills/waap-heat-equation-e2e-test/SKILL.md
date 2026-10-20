---
name: waap-heat-equation-e2e-test
description: Use when end-to-end testing waap by planning, implementing, steering, and verifying a 2D heat equation finite-difference program.
---

# Heat Equation E2E

Use waap as a user would to build a 2D heat equation simulator in a temporary Git repository. Get command usage from the waap skill and CLI documentation.

## Setup

Use the waap binary being tested and authenticated Codex agents. Keep test repositories and runtime paths short:

```sh
export XDG_RUNTIME_DIR=$(mktemp -d /tmp/waap-runtime.XXXXXX)
```

## Specification

Write `specs/spec.md` requiring:

- A runnable CLI using stable explicit finite differences on a square grid of at least 20 by 20 cells.
- An initial hot spot on one edge, with the rest cool; advance until stable.
- A final summary: center and edge temperatures, minimum, maximum, and average.
- A terminal ASCII-art visualization mode.
- A verification command checking that the center warms, the minimum stays non-negative, and the maximum never exceeds the initial hot spot temperature.

## Workflow

1. Initialize the temporary repository and waap state; commit the specification.
2. Create a planning ticket and run a planner to produce implementation tickets.
3. Run developer agents for unblocked tickets, through the visualization implementation.
4. While the visualization developer is active, steer it to **write the final ASCII-art visualization to `final-heatmap.txt` and commit it with its implementation**. Introduce this requirement only through steering, not the initial specification or instructions.
5. Finish the remaining tickets. Run the simulator, its verification command, and visualization mode; compare the visualization with `final-heatmap.txt`.

## Success Criteria

- All tickets and agents complete, and `waap check` passes.
- The simulator satisfies the specification and numerical checks.
- Steering is accepted, and the committed `.txt` contains the actual final visualization—not a synthetic marker.
- Record commands, results, and the test repository path. Report failures honestly and retain artifacts for diagnosis.

Remove the test-created runtime directory after all agents exit.
