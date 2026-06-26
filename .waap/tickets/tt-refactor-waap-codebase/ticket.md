+++
title = "Refactor Waap Codebase"
creation_date = 2026-06-24T10:47:14Z
status = "completed"
+++

# Spec Reference
General code quality and maintainability improvement for the waap implementation, guided by /specs/spec.md and the existing CLI/application behavior.

# Description
Refactor the waap codebase to improve readability, maintainability, testability, and long-term development speed. Use the principles from Robert C. Martin's Clean Code as practical guidance, while applying them pragmatically rather than dogmatically.

The goal is not to rewrite the application or change user-visible behavior. The goal is to leave the existing functionality easier to understand, safer to modify, and better organized for future agents and developers.

# Requirements
- Preserve existing behavior unless a behavior change is explicitly identified, documented, and tested.
- Use meaningful names that reveal intent and avoid misleading abbreviations or vague terms.
- Keep functions focused on one responsibility and one level of abstraction where practical.
- Reduce duplication and consolidate repeated logic when doing so simplifies the design.
- Avoid hidden side effects; make state changes, filesystem writes, and command effects explicit.
- Prefer clear error handling over scattered ad hoc checks.
- Keep comments useful and sparse; comments should explain why, tradeoffs, or non-obvious constraints rather than restating the code.
- Delete dead code, commented-out code, and unused abstractions when found.
- Keep formatting consistent with the existing Rust tooling.
- Improve test coverage around refactored behavior so the refactor is safe.
- Follow the Boy Scout Rule: leave touched code cleaner than it was found.

# Organization Guidance
- Reorganize code into files and modules that are logically grouped by functionality and dependency direction.
- Keep CLI parsing, domain logic, datastore/filesystem operations, serialization/frontmatter handling, and output formatting separated where that reduces coupling.
- Prefer modules that expose cohesive behavior instead of large files containing unrelated command, parsing, validation, and persistence logic.
- Avoid over-engineering: introduce new modules or types only when they clarify ownership, reduce coupling, or make testing easier.

# Dependency Guidance
- Use high-quality, standard dependencies where they can substantially reduce bespoke code that waap must maintain.
- Evaluate existing custom logic, especially date/time handling, slugging, parsing, serialization, and filesystem traversal, to see whether a well-maintained Rust crate would simplify the implementation.
- Only add dependencies when the benefit is clear: less code to maintain, fewer edge cases, stronger correctness, or better interoperability.
- Avoid adding niche dependencies for small savings or where the standard library is sufficient.
- Document any new dependency choice and the code it replaces.

# Clean Code Rules To Consider
- Code is read more often than written; optimize first for clarity.
- Names should reveal intent, be searchable, and be easy to discuss.
- Functions should be small enough to understand and should do one thing.
- Prefer few function arguments; avoid boolean flag arguments when they split behavior.
- Separate commands from queries where practical.
- Keep related code close together.
- Hide internal representation behind behavior-oriented APIs.
- Maintain high cohesion and low coupling.
- Keep tests clean, readable, and behavior-focused.
- Refactor continuously in small, safe steps.

# Suggested Approach
- Inventory the current module/file layout and identify the largest or most coupled areas.
- Identify bespoke utility logic that may be replaceable with standard crates, including date/time logic.
- Refactor incrementally, keeping each change behavior-preserving and covered by tests.
- Run formatting and tests after meaningful refactor steps.

# Validation
- `cargo fmt`
- `cargo test`
- `cargo run -- check`
