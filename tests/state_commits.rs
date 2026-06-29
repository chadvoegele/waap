//! End-to-end tests that the mutating `waap` commands commit their own state changes.

use std::path::Path;
use std::process::{Command, Output, Stdio};

use tempfile::tempdir;

fn git(repo_root: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .current_dir(repo_root)
        .args(args)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "git {args:?} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).trim().to_string()
}

fn init_repo(repo_root: &Path) {
    git(repo_root, &["init", "-q"]);
    git(repo_root, &["config", "user.name", "Test"]);
    git(repo_root, &["config", "user.email", "test@example.com"]);
    // An initial commit so HEAD exists and unrelated history is present.
    std::fs::write(repo_root.join("README.md"), "seed\n").unwrap();
    git(repo_root, &["add", "README.md"]);
    git(repo_root, &["commit", "-q", "-m", "seed"]);
}

fn waap(repo_root: &Path, stdin: &str, args: &[&str]) -> Output {
    use std::io::Write;

    let mut child = Command::new(env!("CARGO_BIN_EXE_waap"))
        .args(["--repo-root", repo_root.to_str().unwrap()])
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

fn commit_count(repo_root: &Path) -> u32 {
    git(repo_root, &["rev-list", "--count", "HEAD"])
        .parse()
        .unwrap()
}

fn last_subject(repo_root: &Path) -> String {
    git(repo_root, &["log", "-1", "--pretty=%s"])
}

fn last_commit_files(repo_root: &Path) -> String {
    git(
        repo_root,
        &["show", "--name-only", "--pretty=format:", "HEAD"],
    )
}

#[test]
fn ticket_new_then_update_each_create_one_commit() {
    let dir = tempdir().unwrap();
    init_repo(dir.path());

    let before = commit_count(dir.path());
    let output = waap(
        dir.path(),
        "# Body\n",
        &["ticket", "new", "--title", "My Task"],
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
    init_repo(dir.path());

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
fn commit_excludes_unrelated_working_tree_changes() {
    let dir = tempdir().unwrap();
    init_repo(dir.path());

    // An unrelated, already-staged user change must not be swept into the waap commit.
    std::fs::write(dir.path().join("user.txt"), "wip\n").unwrap();
    git(dir.path(), &["add", "user.txt"]);

    let output = waap(
        dir.path(),
        "# Body\n",
        &["ticket", "new", "--title", "Task"],
    );
    assert!(output.status.success());

    let files = last_commit_files(dir.path());
    assert!(files.contains(".waap/tickets/tt-task/ticket.md"));
    assert!(!files.contains("user.txt"));
    // The user's staged change survives.
    assert!(git(dir.path(), &["diff", "--cached", "--name-only"]).contains("user.txt"));
}

#[test]
fn failed_commit_returns_error_but_keeps_state() {
    // No git repo: commit must fail, but the state file must still be written.
    let dir = tempdir().unwrap();

    let output = waap(
        dir.path(),
        "# Body\n",
        &["ticket", "new", "--title", "Task"],
    );

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
fn respects_repo_root_run_from_elsewhere() {
    let dir = tempdir().unwrap();
    let repo_root = dir.path().join("project");
    std::fs::create_dir_all(&repo_root).unwrap();
    init_repo(&repo_root);

    // Run the binary with cwd somewhere else; --repo-root must drive git.
    use std::io::Write;
    let other = tempdir().unwrap();
    let mut child = Command::new(env!("CARGO_BIN_EXE_waap"))
        .current_dir(other.path())
        .args(["--repo-root", repo_root.to_str().unwrap()])
        .args(["ticket", "new", "--title", "Task"])
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
    assert_eq!(last_subject(&repo_root), "waap ticket new tt-task");
    assert!(last_commit_files(&repo_root).contains(".waap/tickets/tt-task/ticket.md"));
}

#[test]
fn agent_stop_without_running_agents_creates_no_commit() {
    let dir = tempdir().unwrap();
    init_repo(dir.path());

    let before = commit_count(dir.path());
    let output = waap(dir.path(), "", &["agent", "stop"]);

    assert!(output.status.success());
    assert_eq!(commit_count(dir.path()), before);
}
