+++
title = "Unify ticket and agent name slug ids"
creation_date = 2026-07-05T13:58:55Z
status = "in-progress"
+++

# Goal

Unify ticket and agent creation around an optional human-readable `name` that is slugified into the filesystem id/path.

# Requirements

- `waap ticket new` and `waap agent new` should both accept an optional name argument.
- When a name is provided, slugify it using the same filesystem-safe behavior for both tickets and agents, that is used for tickets currently.
- The slugified name becomes the record id/path component, with the appropriate record prefix/shape preserved as needed by the data model.
- When a name is not provided, generate a random hex id for both ticket and agent, matching the random-hex behavior agents use today.
- Ticket and agent behavior should be consistent for conflicts: if the slugified name id already exists, append a random hex suffix rather than overwriting.
- Preserve existing stdin behavior for ticket and agent markdown content.
- Permit the legacy `title` field in ticket metadata for backward compatibility.
- Map legacy ticket metadata `title` to the new `name` field when reading/reporting tickets.
- Prefer writing new ticket metadata with `name` rather than `title`.

# Questions To Resolve During Implementation

- Confirm exact CLI shape before coding if ambiguous: likely `--name`, replacing ticket `--title` for new writes while accepting legacy metadata on read only.
- Confirm whether generated random ticket ids should use the same 8-hex style generalized to the ticket prefix, e.g. `tt-<8 hex>`.
- Confirm whether slugified names allow underscores for both ticket and agent ids.

# Validation

Run the project validations required by `AGENTS.md` before committing:

```sh
cargo clippy --all-targets -- -D warnings
cargo fmt --check
cargo build
cargo build --release
cargo test
```
