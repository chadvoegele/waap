+++
name = "CODE.md quality refactor gpt-6.1-sol"
creation_date = 2026-10-04T10:56:09Z
status = "ready"
+++

Refactor https://github.com/chadvoegele/waap for the highest practical code quality, using `/home/chad/code/code.voegele.me/chad/agents/CODE.md` as the governing design guidance. Deliver tested code and a review-ready pull request, not just recommendations. Do not merge.

## 1. Understand the existing system

Read CODE.md completely, repository instructions, documentation, source, and tests. Use the git-repositories skill to locate the repository.

Inventory waap’s actual functionality and contracts, including:
- CLI commands, options, output formats, configuration, and exit behavior.
- Ticket records, statuses, dependencies, DAG validation, and eligibility.
- Agent records, execution backends, sessions, and lifecycle.
- State storage, Git persistence, initialization, migration, and repair.
- Worktree creation, isolation, cleanup, failure recovery, and concurrency.

Trace representative operations end to end. Identify undocumented behavior and test gaps. Distinguish intentional contracts from accidental implementation details.

Before refactoring, add characterization tests where existing tests do not protect important behavior. Do not treat existing defects as contracts; document and test any deliberate correction.

## 2. Design as though starting from scratch

Independently design the simplest architecture that would implement the observed requirements while adhering to every guideline in CODE.md. Then compare it with the current implementation and choose an incremental path toward that design. This is a design exercise, not permission for an unnecessary rewrite.

Produce:
- A functionality and compatibility inventory.
- A directional diagram of 5–10 cohesive components.
- Smaller diagrams for complex components, preserving the same clarity when zooming in.
- Component responsibilities, interfaces, invariants, ownership, and failure handling.
- A brief comparison of alternatives and reasons for the chosen design.
- A checklist mapping every CODE.md guideline to a concrete decision, verification, or justified exception.

Apply all guidance explicitly:

### Design and abstractions
- Prefer composable components with interchangeable behaviors.
- Group code by functionality, not by type.
- Prefer immutable data and explicit state transitions.
- Remove dead code and repair defects encountered in affected paths.
- Use data structures when new operations are the likely extension axis; interfaces when new implementations are the likely extension axis.
- Minimize what components know about each other.
- Keep implementation details behind clear contracts.
- Avoid speculative abstractions and unnecessary layers.

### Scope and naming
- Keep variables, functions, methods, and constants in the smallest useful scope.
- Expose only what callers need.
- Use descriptive names, verb-led function names, consistent existing terminology, and named values instead of magic numbers.

### Responsibilities and effects
- Give each component one clear responsibility and explicit handoffs.
- Check preconditions early.
- Keep functions focused.
- Separate decisions and transformations from side effects; keep effectful operations narrowly focused.
- Make resource ownership and cleanup explicit, including partial failure.

### Comments and dependencies
- Prefer readable code to explanatory comments.
- Retain comments only for necessary rationale or non-obvious constraints.
- Keep work logs out of production code; use Git history and waap records.
- Treat dependencies as maintained parts of the stack.
- Prefer open-source dependencies; justify additions and avoid unnecessary ones.

### Errors and maintainability
- Make user-facing errors actionable.
- Preserve enough internal context to diagnose root causes without exposing secrets.
- Eliminate vague names, duplication, long functions, oversized interfaces and records, long parameter lists, unnecessary globals, excessive mutation, inconsistent objects, missing cleanup, and changes that ripple across unrelated components.
- Optimize for clear reasoning and maintainability, not cleverness or arbitrary size metrics.

### Tests
- Test all changed behavior.
- Use mostly unit tests, integration tests for component contracts, and focused end-to-end tests for user workflows.
- Consult the references named in CODE.md where useful; do not claim to have consulted material you have not read.

## 3. Implement the refactor

Work on a dedicated branch in an isolated worktree, respecting waap’s managed worktree lifecycle. Inspect repository status first and preserve unrelated changes.

Implement the design in coherent, reviewable steps. Prefer behavior-preserving changes. Preserve public CLI behavior, persisted records, configuration, and existing workflows unless a documented defect requires correction.

Pay particular attention to:
- Deterministic validation and state transitions.
- Persistence integrity and concurrent access.
- Git and subprocess failure handling.
- Worktree cleanup after success, failure, and interruption.
- Execution-backend boundaries.
- Testability without real external agents, network access, or destructive operations.

Do not add speculative features, unrelated cleanup, or abstractions without a current use. Update documentation to describe the implemented architecture and any intentional behavior changes.

Maintain a concise work log in your waap agent records, not source comments. Record progress, decisions, test results, and unresolved risks.

## 4. Verify and review

Run the repository’s required checks, including:
- `cargo fmt --check`
- `cargo clippy` with the repository’s prescribed options
- `cargo test`
- Relevant end-to-end workflows following repository instructions

Add coverage for changed contracts, malformed state, missing dependencies, cycles, failed processes, partial writes, cleanup, and concurrency where relevant.

Use isolated test repositories and state directories. Do not mutate real user workflows to test the refactor.

Review the final diff against every CODE.md guideline and the functionality inventory. Remove obsolete paths, temporary scaffolding, and redundant tests. Check for regressions, unnecessary API expansion, and unhandled failure paths.

Report exact commands and results. Clearly identify anything not verified; do not claim success for checks that were not run.

## 5. Open the review request

Push the branch and open a GitHub pull request against the repository’s default branch. Request review from Chad using the verified GitHub account; do not guess the username.

Mark the PR description and human-facing comments as “Chad's Agent”.

Include:
- The architectural problems addressed.
- The new component diagram and key design decisions.
- Compatibility guarantees and intentional changes.
- Testing performed and results.
- Remaining limitations or risks.
- A concise summary of adherence to CODE.md, including justified exceptions.

Keep the PR focused and reviewable. If the necessary scope is large, use logical commits and clearly explain their sequence.

Do not merge. Stop after opening the PR and report its URL, branch, test results, and any blockers. Wait for Chad’s review and explicit approval.
