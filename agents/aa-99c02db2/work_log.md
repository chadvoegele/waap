# Work Log

- Read the agent instructions, ticket, and waap workflow.
- Marked `tt-isolate-tests-from-user-git-configuration` in progress.
- Began inventorying unit and integration tests that invoke Git directly or through the waap binary.
- Reproduced the CI failure by running `agent_branch_rebase_and_ff_merge_keeps_main_linear` with an external `init.defaultBranch=master`; `git rebase main` failed because the fixture created `master`.
- Added shared unit-test and integration-test Git helpers. Each command ignores system/global config, supplies deterministic identity, disables signing and hooks, and creates repositories with `--initial-branch=main`.
- Routed Git commands inside unit-tested production helpers through the test isolation without changing non-test behavior.
- Added a regression test with hostile default-branch, signing, and hooks settings.
- Verified the regression test, formerly failing branch test, and `state_commits` integration suite pass.
- Passed clippy with warnings denied, formatting, debug and release builds, and the full test suite.
- Passed the full test suite with hostile system/global configuration selecting `master`, enabling signing, changing identity, and installing a failing pre-commit hook.
