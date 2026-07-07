+++
name = "Refactor ticket dependency checks around parsed metadata"
creation_date = 2026-07-07T11:59:59Z
status = "completed"
depends_on = ["tt-move-agent-frontmatter-check-into-check-module"]
+++

# Goal

Refactor `src/check.rs` ticket dependency validation to be more readable by collecting valid parsed ticket metadata first, then running named dependency checks over that list.

# Background

`check_tickets(tickets_dir: &Path, errors: &mut Vec<String>)` currently constructs `known_ids: HashSet<String>` and `deps_map: HashMap<String, Vec<String>>` while scanning ticket directories. It then passes those maps into `check_ticket_dependencies()`, which performs two distinct checks inline:

- missing dependency targets
- dependency cycles

This is efficient, but the control flow mixes directory validation, frontmatter parsing, dependency indexing, missing-target validation, and cycle detection in one path.

# Desired Behavior

Keep the existing validation behavior, but restructure for readability:

- During `check_tickets()`, collect a list of valid parsed ticket metadata records in memory.
- Add the ticket id to `TicketMetadata`, so each parsed metadata value carries the id associated with its directory.
- In the current frontmatter parsing branch, instead of immediately populating `deps_map`, push the successfully parsed metadata with its ticket id into the collected list.
- Move construction of derived structures such as known ids and dependency maps lower, inside dependency-check helper functions or immediately before they are needed.
- Change `check_ticket_dependencies(...)` to accept the list of parsed ticket metadata rather than prebuilt `known_ids` / `deps_map` maps.
- Split the two dependency validations into separate helpers:
  - `check_dependencies_exist(...)` for missing `depends_on` targets.
  - `check_cycles(...)` for cycle detection.
- `check_dependencies_exist(...)` and `check_cycles(...)` should both operate from the parsed ticket metadata list, even if that does a little redundant map/set construction. Prefer readability over micro-optimization here.
- Preserve existing error messages unless a minor wording change is necessary.

# Implementation Notes

Relevant current code in `src/check.rs`:

```rust
let mut known_ids: HashSet<String> = HashSet::new();
let mut deps_map: HashMap<String, Vec<String>> = HashMap::new();
```

and:

```rust
match TicketMetadata::from_frontmatter(&frontmatter, &ticket_file) {
    Ok(metadata) => {
        if let Some(deps) = metadata.depends_on {
            deps_map.insert(name.clone(), deps);
        }
    }
    Err(mut frontmatter_errors) => errors.append(&mut frontmatter_errors),
}
```

The desired shape is to keep scanning logic largely intact, but store successfully parsed metadata instead of manually updating dependency maps at that point.

Be careful that adding `ticket_id` to `TicketMetadata` may affect creation, update, serialization, and tests. If making `ticket_id` required on all `TicketMetadata` values is too invasive, use the smallest clean alternative that still gives the dependency checks a list of parsed metadata paired with ticket ids, and document the choice in code or tests.

# Acceptance Criteria

- `check_tickets()` no longer maintains top-level `known_ids` and `deps_map` variables while scanning entries.
- Successfully parsed ticket metadata is collected in memory with the associated ticket id.
- `check_ticket_dependencies()` accepts that metadata list, not separate `known_ids` and `deps_map` arguments.
- Missing dependency validation lives in a helper named `check_dependencies_exist()`.
- Cycle validation lives in a helper named `check_cycles()`.
- Existing missing-dependency and cycle-detection behavior remains covered by tests.
- Existing valid and invalid ticket frontmatter behavior remains unchanged.
- Developer validations pass:
  - `cargo clippy --all-targets -- -D warnings`
  - `cargo fmt --check`
  - `cargo build`
  - `cargo build --release`
  - `cargo test`
