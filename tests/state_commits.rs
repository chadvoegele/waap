//! End-to-end tests that the mutating `waap` commands commit their own state changes.

mod common;

use std::path::Path;
use std::process::{Command, Output, Stdio};

use tempfile::tempdir;

use common::{git, init_repo, isolate_git_config};

/// Initialize a git repo and an already-initialized waap project inside it.
fn init_repo_with_waap_project(waap_root: &Path) {
    init_repo(waap_root);
    let output = waap(waap_root, "", &["init"]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn waap(waap_root: &Path, stdin: &str, args: &[&str]) -> Output {
    use std::io::Write;

    let mut command = Command::new(env!("CARGO_BIN_EXE_waap"));
    isolate_git_config(&mut command);
    let mut child = command
        .args(["--waap-root", waap_root.to_str().unwrap()])
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(stdin.as_bytes())
        .unwrap();
    child.wait_with_output().unwrap()
}

fn commit_count(waap_root: &Path) -> u32 {
    git(waap_root, &["rev-list", "--count", "HEAD"])
        .parse()
        .unwrap()
}

fn last_subject(waap_root: &Path) -> String {
    git(waap_root, &["log", "-1", "--pretty=%s"])
}

fn last_commit_files(waap_root: &Path) -> String {
    git(
        waap_root,
        &["show", "--name-only", "--pretty=format:", "HEAD"],
    )
}

#[test]
fn ticket_new_then_update_each_create_one_commit() {
    let dir = tempdir().unwrap();
    init_repo_with_waap_project(dir.path());

    let before = commit_count(dir.path());
    let output = waap(
        dir.path(),
        "# Body\n",
        &["ticket", "new", "--name", "My Task"],
    );
    assert!(output.status.success());
    assert_eq!(commit_count(dir.path()), before + 1);
    assert_eq!(last_subject(dir.path()), "waap ticket new tt-my-task");
    assert!(last_commit_files(dir.path()).contains(".waap/tickets/tt-my-task/ticket.md"));

    let output = waap(
        dir.path(),
        "",
        &[
            "ticket",
            "update",
            "--ticket-id",
            "tt-my-task",
            "--set-status",
            "in-progress",
        ],
    );
    assert!(output.status.success());
    assert_eq!(commit_count(dir.path()), before + 2);
    assert_eq!(last_subject(dir.path()), "waap ticket update tt-my-task");
}

#[test]
fn agent_new_then_update_each_create_one_commit() {
    let dir = tempdir().unwrap();
    init_repo_with_waap_project(dir.path());

    let before = commit_count(dir.path());
    let output = waap(
        dir.path(),
        "# Purpose\n",
        &["--output-format", "json", "agent", "new"],
    );
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    let value: serde_json::Value = serde_json::from_str(stdout.trim()).unwrap();
    let agent_id = value["agent_id"].as_str().unwrap().to_string();
    // JSON output indicates the commit hash.
    let commit = value["commit"].as_str().unwrap();
    assert_eq!(commit, git(dir.path(), &["rev-parse", "HEAD"]));

    assert_eq!(commit_count(dir.path()), before + 1);
    assert_eq!(
        last_subject(dir.path()),
        format!("waap agent new {agent_id}")
    );

    let output = waap(
        dir.path(),
        "",
        &[
            "agent",
            "update",
            "--agent-id",
            &agent_id,
            "--set-status",
            "completed",
        ],
    );
    assert!(output.status.success());
    assert_eq!(commit_count(dir.path()), before + 2);
    assert_eq!(
        last_subject(dir.path()),
        format!("waap agent update {agent_id}")
    );
}

#[test]
fn agent_new_with_name_creates_slug_id() {
    let dir = tempdir().unwrap();
    init_repo_with_waap_project(dir.path());

    let output = waap(
        dir.path(),
        "# Purpose\n",
        &[
            "--output-format",
            "json",
            "agent",
            "new",
            "--name",
            "Custom Agent_123",
        ],
    );

    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    let value: serde_json::Value = serde_json::from_str(stdout.trim()).unwrap();
    assert_eq!(value["agent_id"], "aa-custom-agent123");
    assert_eq!(value["metadata"]["name"], "Custom Agent_123");
    assert!(dir
        .path()
        .join(".waap/agents/aa-custom-agent123/agent.md")
        .is_file());
    assert_eq!(
        last_subject(dir.path()),
        "waap agent new aa-custom-agent123"
    );
}

#[test]
fn commit_excludes_unrelated_working_tree_changes() {
    let dir = tempdir().unwrap();
    init_repo_with_waap_project(dir.path());

    // An unrelated, already-staged user change must not be swept into the waap commit.
    std::fs::write(dir.path().join("user.txt"), "wip\n").unwrap();
    git(dir.path(), &["add", "user.txt"]);

    let output = waap(dir.path(), "# Body\n", &["ticket", "new", "--name", "Task"]);
    assert!(output.status.success());

    let files = last_commit_files(dir.path());
    assert!(files.contains(".waap/tickets/tt-task/ticket.md"));
    assert!(!files.contains("user.txt"));
    // The user's staged change survives.
    assert!(git(dir.path(), &["diff", "--cached", "--name-only"]).contains("user.txt"));
}

#[test]
fn failed_commit_returns_error_but_keeps_state() {
    // Force git's index to be locked: commit must fail, but the state file must still be written.
    let dir = tempdir().unwrap();
    init_repo(dir.path());
    std::fs::create_dir_all(dir.path().join(".waap")).unwrap();
    std::fs::File::create(dir.path().join(".git/index.lock")).unwrap();

    let output = waap(dir.path(), "# Body\n", &["ticket", "new", "--name", "Task"]);

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("failed to commit waap state change"),
        "{stderr}"
    );
    // State update is intact on disk despite the commit failure.
    assert!(dir.path().join(".waap/tickets/tt-task/ticket.md").is_file());
}

#[test]
fn respects_waap_root_run_from_elsewhere() {
    let dir = tempdir().unwrap();
    let waap_root = dir.path().join("project");
    std::fs::create_dir_all(&waap_root).unwrap();
    init_repo_with_waap_project(&waap_root);

    // Run the binary with cwd somewhere else; --waap-root must drive git.
    use std::io::Write;
    let other = tempdir().unwrap();
    let mut command = Command::new(env!("CARGO_BIN_EXE_waap"));
    isolate_git_config(&mut command);
    let mut child = command
        .current_dir(other.path())
        .args(["--waap-root", waap_root.to_str().unwrap()])
        .args(["ticket", "new", "--name", "Task"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(b"# Body\n").unwrap();
    let output = child.wait_with_output().unwrap();

    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(last_subject(&waap_root), "waap ticket new tt-task");
    assert!(last_commit_files(&waap_root).contains(".waap/tickets/tt-task/ticket.md"));
}

#[test]
fn agent_stop_without_running_agents_creates_no_commit() {
    let dir = tempdir().unwrap();
    init_repo_with_waap_project(dir.path());

    let before = commit_count(dir.path());
    let output = waap(dir.path(), "", &["agent", "stop"]);

    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(commit_count(dir.path()), before);
}

#[test]
fn init_creates_and_commits_waap_skeleton() {
    let dir = tempdir().unwrap();
    init_repo(dir.path());

    let before = commit_count(dir.path());
    let output = waap(dir.path(), "", &["--output-format", "json", "init"]);

    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    let value: serde_json::Value = serde_json::from_str(stdout.trim()).unwrap();
    let commit = value["commit"].as_str().unwrap();
    assert_eq!(commit, git(dir.path(), &["rev-parse", "HEAD"]));

    assert_eq!(commit_count(dir.path()), before + 1);
    assert_eq!(last_subject(dir.path()), "waap init");
    assert!(dir.path().join(".waap/agents").is_dir());
    assert!(dir.path().join(".waap/tickets").is_dir());
}

#[test]
fn init_errors_when_waap_already_exists() {
    let dir = tempdir().unwrap();
    init_repo_with_waap_project(dir.path());

    let output = waap(dir.path(), "", &["init"]);

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains(".waap"), "{stderr}");
}

#[test]
fn init_errors_outside_git_repository() {
    let dir = tempdir().unwrap();

    let output = waap(dir.path(), "", &["init"]);

    assert!(!output.status.success());
    assert!(!dir.path().join(".waap").exists());
}

#[test]
fn ticket_new_errors_when_project_not_initialized() {
    let dir = tempdir().unwrap();
    init_repo(dir.path());

    let output = waap(dir.path(), "# Body\n", &["ticket", "new", "--name", "Task"]);

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("waap init"), "{stderr}");
    assert!(!dir.path().join(".waap").exists());
}

#[test]
fn agent_new_errors_when_project_not_initialized() {
    let dir = tempdir().unwrap();
    init_repo(dir.path());

    let output = waap(dir.path(), "# Purpose\n", &["agent", "new"]);

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("waap init"), "{stderr}");
    assert!(!dir.path().join(".waap").exists());
}
