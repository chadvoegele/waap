+++
name = "Move TOML helpers out of ids/frontmatter"
creation_date = 2026-07-08T10:43:19Z
status = "pending"
+++

## Context

During inspection, TOML serialization helpers were found in modules that do not match their responsibility:

- `src/ids.rs` defines `current_toml_datetime()` and `toml_string()`, but `ids.rs` otherwise handles record IDs and slug generation.
- `src/frontmatter.rs` defines `datetime_string()`, which is a generic TOML `Value::Datetime` extraction helper, while the rest of the file focuses on frontmatter parsing and validation.
- Agent and ticket metadata serialization both use these TOML helpers.

## Proposed Change

Create a focused `src/toml.rs` module for shared TOML helpers. Disambiguate the local module from the external crate with explicit paths: use `crate::toml::{...}` for local helpers and `::toml::Value` where the external type is needed.

- Move `current_toml_datetime()` from `ids.rs` to `toml.rs`.
- Move `toml_string()` from `ids.rs` to `toml.rs`.
- Move `datetime_string()` from `frontmatter.rs` to `toml.rs` if the final inspection still confirms it is not frontmatter-specific.
- Add `mod toml;` in `src/main.rs`.
- Update imports in agent and ticket modules.

Keep `src/frontmatter.rs` responsible for TOML frontmatter delimiters, parsing, error construction, and field validation.
Keep `src/ids.rs` responsible for IDs, slugs, and record ID validation.

## Validation

Run the standard developer validations from the repository root:

```sh
cargo clippy --all-targets -- -D warnings
cargo fmt --check
cargo build
cargo build --release
cargo test
```

## Notes

Prefer the smallest correct refactor. Do not introduce backward-compatibility aliases unless needed by actual external consumers.
