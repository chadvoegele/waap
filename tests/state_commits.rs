//! End-to-end tests for state mutations on the dedicated `waap` branch.

mod common;

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

use tempfile::tempdir;

use common::{git, init_repo, isolate_git_config};

fn waap(cwd: &Path, home: &Path, stdin: &str, args: &[&str]) -> Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_waap"));
    isolate_git_config(&mut command);
    let mut child = command
        .current_dir(cwd)
        .env("HOME", home)
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

fn state_root(home: &Path, repository: &Path) -> PathBuf {
    home.join(".local/state/waap/data").join(
        repository
            .canonicalize()
            .unwrap()
            .strip_prefix("/")
            .unwrap(),
    )
}

fn init_state(repository: &Path, home: &Path) -> PathBuf {
    let output = waap(repository, home, "", &["init"]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    state_root(home, repository)
}

fn head(root: &Path) -> String {
    git(root, &["rev-parse", "HEAD"])
}

#[test]
fn ticket_mutations_from_linked_worktrees_commit_only_central_state() {
    let repository = tempdir().unwrap();
    let home = tempdir().unwrap();
    init_repo(repository.path());
    let state = init_state(repository.path(), home.path());
    let first = repository.path().join("first");
    let second = repository.path().join("second");
    git(
        repository.path(),
        &["worktree", "add", first.to_str().unwrap(), "-b", "first"],
    );
    git(
        repository.path(),
        &["worktree", "add", second.to_str().unwrap(), "-b", "second"],
    );
    let application_heads = [head(repository.path()), head(&first), head(&second)];
    let state_before = head(&state);

    let created = waap(
        &first,
        home.path(),
        "# Body\n",
        &[
            "--output-format",
            "json",
            "ticket",
            "new",
            "--name",
            "Shared task",
        ],
    );
    assert!(
        created.status.success(),
        "{}",
        String::from_utf8_lossy(&created.stderr)
    );
    let created: serde_json::Value = serde_json::from_slice(&created.stdout).unwrap();
    assert_eq!(created["commit"], head(&state));
    assert_ne!(head(&state), state_before);
    assert_eq!(
        git(&state, &["show", "--format=", "--name-only", "HEAD"]),
        "tickets/tt-shared-task/ticket.md"
    );
    assert_eq!(
        application_heads,
        [head(repository.path()), head(&first), head(&second)]
    );

    let listed = waap(
        &second,
        home.path(),
        "",
        &["ticket", "list", "--status", "pending"],
    );
    assert!(
        listed.status.success(),
        "{}",
        String::from_utf8_lossy(&listed.stderr)
    );
    assert!(String::from_utf8_lossy(&listed.stdout).contains("tt-shared-task"));

    let updated = waap(
        repository.path(),
        home.path(),
        "",
        &[
            "--output-format",
            "json",
            "ticket",
            "update",
            "--ticket-id",
            "tt-shared-task",
            "--set-status",
            "in-progress",
        ],
    );
    assert!(
        updated.status.success(),
        "{}",
        String::from_utf8_lossy(&updated.stderr)
    );
    let updated: serde_json::Value = serde_json::from_slice(&updated.stdout).unwrap();
    assert_eq!(updated["commit"], head(&state));
    assert_eq!(
        git(&state, &["log", "-1", "--format=%s"]),
        "waap ticket update tt-shared-task"
    );
    assert_eq!(
        application_heads,
        [head(repository.path()), head(&first), head(&second)]
    );
}

#[test]
fn agent_mutations_use_the_explicit_central_state_root() {
    let repository = tempdir().unwrap();
    let home = tempdir().unwrap();
    init_repo(repository.path());
    let state = init_state(repository.path(), home.path());
    let application_head = head(repository.path());

    let output = waap(
        repository.path(),
        home.path(),
        "# Purpose\nDo work\n",
        &[
            "--waap-root",
            state.to_str().unwrap(),
            "--output-format",
            "json",
            "agent",
            "new",
            "--name",
            "Central agent",
        ],
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let output: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(output["commit"], head(&state));
    assert!(state.join("agents/aa-central-agent/agent.md").is_file());
    assert_eq!(head(repository.path()), application_head);
}
