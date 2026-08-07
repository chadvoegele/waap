+++
title = "Isolate tests from user Git configuration"
creation_date = 2026-07-06T15:03:59Z
status = "completed"
+++

## Problem

Git-backed tests inherit the developer or CI runner's system and user Git configuration. This caused GitHub Actions run 28798788398 to fail `agent::run::tests::agent_branch_rebase_and_ff_merge_keeps_main_linear`: the fixture used plain `git init`, then assumed the initial branch was `main`. It passed locally because the user's Git configuration selected `main`, but CI selected `master`.

Other inherited settings, including identity, signing, hooks, aliases, and default branch, can cause similar nondeterminism.

## Requirements

- Run every unit and integration test that invokes Git, directly or through the waap binary, with fresh isolated system and global Git configuration.
- Set all required Git behavior explicitly, including the initial branch and commit identity.
- Preserve normal Git configuration behavior outside tests.
- Avoid process-global environment races when tests run in parallel.
- Consolidate duplicated Git test setup where practical.

## Acceptance criteria

- Tests do not read the invoking user's system or global Git configuration.
- Temporary repositories use an explicitly selected initial branch rather than Git's configured default.
- The full suite passes when the invoking environment contains conflicting settings such as `init.defaultBranch=master`, commit signing, or a custom hooks path.
- A regression test demonstrates that external Git configuration cannot change test results.
- `cargo clippy --all-targets -- -D warnings`, `cargo fmt --check`, `cargo build`, `cargo build --release`, and `cargo test` pass.

## Context

- Failing workflow: https://github.com/chadvoegele/waap/actions/runs/28798788398
- Current fixture: `src/agent/run.rs::init_repo_with_commit`
- Current failing assumption: `git rebase main` in `agent_branch_rebase_and_ff_merge_keeps_main_linear`
