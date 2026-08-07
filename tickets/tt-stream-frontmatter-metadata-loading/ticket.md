+++
name = "Stream frontmatter metadata loading"
creation_date = 2026-07-08T12:12:53Z
status = "completed"
depends_on = ["tt-move-toml-helpers-out-of-idsfrontmatter"]
+++

## Context

Agent and ticket metadata loaders currently read the entire Markdown record into memory before parsing TOML frontmatter:

- `load_agent_metadata()` uses `fs::read_to_string()` in `src/agent.rs`.
- `load_ticket_metadata()` uses `fs::read_to_string()` in `src/ticket.rs`.
- `parse_frontmatter_from_contents()` only scans until the second `+++`, but callers have already loaded the whole file.

For metadata-only reads, this is unnecessary. The frontmatter parser can stop as soon as it reaches the closing `+++` delimiter.

There is also duplicated metadata parsing logic between:

- `load_agent_metadata()` and `read_agent_record()`.
- `load_ticket_metadata()` and `read_ticket_record()`.

## Proposed Change

Refactor frontmatter metadata loading in two steps.

### 1. Stream Frontmatter Parsing

Change the generic frontmatter file parser in `src/frontmatter.rs` so metadata-only parsing reads only the frontmatter section:

- Update `parse_frontmatter(path, errors)` to open the file with a buffered reader.
- Read line-by-line.
- Require the first line to be `+++`.
- Accumulate TOML lines until the second `+++`.
- Stop reading immediately after the closing delimiter.
- Preserve current error messages and validation behavior where practical.
- Keep `parse_frontmatter_from_contents()` for callers/tests that already have full contents.

This means metadata-only loads should not read record bodies into memory.

### 2. Reuse Frontmatter Loader In Record Metadata Loaders

Update metadata loaders to use the generic frontmatter file parser:

- `load_agent_metadata()` should use `parse_frontmatter(&path, &mut errors)`.
- `load_ticket_metadata()` should use `parse_frontmatter(&path, &mut errors)`.

Then simplify full record readers as discussed:

- `read_agent_record()` can read the full file for the body, then call `load_agent_metadata()` for metadata.
- `read_ticket_record()` can read the full file for the body, then call `load_ticket_metadata()` for metadata.

This intentionally accepts a second read in the full-record path because metadata reads only the frontmatter section after this refactor, while full-record reads still need the whole body.

## Constraints

- Keep the smallest correct change.
- Do not introduce a separate private read-once helper unless the implementation clearly needs it.
- Do not change record file format.
- Preserve existing frontmatter validation semantics.

## Tests

Add or update tests for:

- `parse_frontmatter(path, errors)` stops after the closing delimiter. Prove this with invalid UTF-8 bytes after a valid frontmatter section, or an equally direct read-boundary test; merely putting non-TOML Markdown in the body does not prove streaming.
- Missing opening delimiter still reports the current missing-frontmatter error.
- Missing closing delimiter still reports the current missing-closing-delimiter error.
- Agent and ticket metadata loading still works.
- `read_agent_record()` and `read_ticket_record()` still return metadata plus markdown body correctly.

## Validation

Run from the repository root:

```sh
cargo clippy --all-targets -- -D warnings
cargo fmt --check
cargo build
cargo build --release
cargo test
```
