+++
name = "Reuse Agent Record Metadata For Content Reports"
creation_date = 2026-07-09T02:33:56Z
status = "completed"
+++

# Summary

Refactor agent content loading to avoid parsing agent metadata twice.

# Problem

`load_agent_content` currently calls `load_agent_report`, which loads metadata, then calls `read_agent_record`, which loads metadata again while also returning the markdown body.

```rust
let report = load_agent_report(waap_root, agent_id)?;
let (_, body) = read_agent_record(waap_root, agent_id)?;
```

This duplicates frontmatter parsing and discards metadata already returned by `read_agent_record`.

# Desired Change

Create a shared helper that constructs an `AgentReport` from already-loaded `AgentMetadata`.

Use that helper from both:

- `load_agent_report`, after loading metadata with `load_agent_metadata`
- `load_agent_content`, after loading `(metadata, body)` with `read_agent_record`

This should keep `load_agent_report` from reading the markdown body while allowing `load_agent_content` to parse metadata only once.

# Suggested Implementation

In `src/agent/get.rs`, introduce a private helper similar to:

```rust
fn agent_report_from_metadata(
    waap_root: &Path,
    agent_id: &str,
    metadata: AgentMetadata,
) -> io::Result<AgentReport> {
    let path = agent_path(waap_root, agent_id);
    let file_size = fs::metadata(&path)?.len();

    Ok(AgentReport {
        agent_id: agent_id.to_string(),
        path,
        metadata,
        file_size,
    })
}
```

Then update `load_agent_report` and `load_agent_content` to call it.

# Acceptance Criteria

- `load_agent_content` uses the metadata returned from `read_agent_record` instead of discarding it.
- `load_agent_report` uses the same helper to create `AgentReport`.
- Existing behavior and report fields remain unchanged.
- Existing tests pass; add or adjust tests only if needed.

# Validation

Run the repository validations from `AGENTS.md` if implementing:

```sh
cargo clippy --all-targets -- -D warnings
cargo fmt --check
cargo build
cargo build --release
cargo test
```
