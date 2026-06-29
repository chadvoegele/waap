//! Tests that the agent instruction templates no longer tell agents to manage their own worktree.
//!
//! `waap agent run` now owns the worktree lifecycle, so the generated agent instructions (the
//! reusable role templates and the spec they document) must not instruct agents to create or remove
//! a worktree themselves.

use std::path::Path;

fn read(relative: &str) -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join(relative);
    std::fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("failed to read {}: {error}", path.display()))
}

#[test]
fn developer_template_has_no_worktree_lifecycle_instructions() {
    let template = read(".agents/skills/waap/roles/developer/agent.md");

    assert!(
        !template.contains("git worktree add"),
        "developer template should not tell the agent to create a worktree"
    );
    assert!(
        !template.contains("git worktree remove"),
        "developer template should not tell the agent to remove a worktree"
    );
}

#[test]
fn spec_developer_role_has_no_worktree_lifecycle_instructions() {
    let spec = read("specs/spec.md");
    let marker = "### .agents/skills/waap/roles/developer/agent.md";
    let role = spec
        .split_once(marker)
        .map(|(_, rest)| rest)
        .and_then(|rest| rest.split_once("\n### "))
        .map(|(role, _)| role)
        .unwrap_or_else(|| panic!("developer role section not found in spec.md"));

    assert!(
        !role.contains("git worktree add"),
        "spec developer role should not tell the agent to create a worktree"
    );
    assert!(
        !role.contains("git worktree remove"),
        "spec developer role should not tell the agent to remove a worktree"
    );
}
