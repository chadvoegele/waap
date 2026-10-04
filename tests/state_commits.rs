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

fn state_root(repository_root: &Path) -> std::path::PathBuf {
    repository_root.join(".state")
}

fn waap(waap_root: &Path, stdin: &str, args: &[&str]) -> Output {
    use std::io::{ErrorKind, Write};

    let state_root = state_root(waap_root);
    let mut command = Command::new(env!("CARGO_BIN_EXE_waap"));
    isolate_git_config(&mut command);
    let mut child = command
        .current_dir(waap_root)
        .args(["--waap-root", state_root.to_str().unwrap()])
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let write_result = child.stdin.take().unwrap().write_all(stdin.as_bytes());
    if let Err(error) = write_result {
        assert_eq!(
            error.kind(),
            ErrorKind::BrokenPipe,
            "stdin write failed: {error}"
        );
    }
    child.wait_with_output().unwrap()
}

fn commit_count(waap_root: &Path) -> u32 {
    git(&state_root(waap_root), &["rev-list", "--count", "HEAD"])
        .parse()
        .unwrap()
}

fn last_subject(waap_root: &Path) -> String {
    git(&state_root(waap_root), &["log", "-1", "--pretty=%s"])
}

fn last_commit_files(waap_root: &Path) -> String {
    git(
        &state_root(waap_root),
        &["show", "--name-only", "--pretty=format:", "HEAD"],
    )
}

#[test]
fn multiline_names_cannot_close_record_frontmatter() {
    let dir = tempdir().unwrap();
    init_repo_with_waap_project(dir.path());
    let name = "a\n+++\nb";
    for (kind, id_option, id, status) in [
        ("ticket", "--ticket-id", "tt-a-b", "completed"),
        ("agent", "--agent-id", "aa-a-b", "aborted"),
    ] {
        let created = waap(dir.path(), "# Body\n", &[kind, "new", "--name", name]);
        assert!(created.status.success(), "{created:?}");
        let updated = waap(
            dir.path(),
            "",
            &[kind, "update", id_option, id, "--set-status", status],
        );
        assert!(updated.status.success(), "{updated:?}");
        let output = waap(
            dir.path(),
            "",
            &["--output-format", "json", kind, "get", id_option, id],
        );
        assert!(output.status.success(), "{output:?}");
        let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(report["metadata"]["name"], name);
        assert!(report["content"].as_str().unwrap().ends_with("# Body\n"));
        assert!(waap(dir.path(), "", &["check"]).status.success());
    }
}

#[test]
fn ticket_new_then_update_each_create_one_commit() {
    let dir = tempdir().unwrap();
    init_repo_with_waap_project(dir.path());

    let before = commit_count(dir.path());
    let output = waap(
        dir.path(),
        "# Body\n",
        &[
            "--output-format",
            "json",
            "ticket",
            "new",
            "--name",
            "My Task",
        ],
    );
    assert!(output.status.success());
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(
        value["commit"],
        git(&state_root(dir.path()), &["rev-parse", "HEAD"])
    );
    assert_eq!(commit_count(dir.path()), before + 1);
    assert_eq!(last_subject(dir.path()), "waap ticket new tt-my-task");
    assert!(last_commit_files(dir.path()).contains("tickets/tt-my-task/ticket.md"));

    let output = waap(
        dir.path(),
        "",
        &[
            "--output-format",
            "json",
            "ticket",
            "update",
            "--ticket-id",
            "tt-my-task",
            "--set-status",
            "in-progress",
        ],
    );
    assert!(output.status.success());
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(
        value["commit"],
        git(&state_root(dir.path()), &["rev-parse", "HEAD"])
    );
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
    assert_eq!(commit, git(&state_root(dir.path()), &["rev-parse", "HEAD"]));

    assert_eq!(commit_count(dir.path()), before + 1);
    assert_eq!(
        last_subject(dir.path()),
        format!("waap agent new {agent_id}")
    );

    let output = waap(
        dir.path(),
        "",
        &[
            "--output-format",
            "json",
            "agent",
            "update",
            "--agent-id",
            &agent_id,
            "--set-status",
            "running",
        ],
    );
    assert!(output.status.success());
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(
        value["commit"],
        git(&state_root(dir.path()), &["rev-parse", "HEAD"])
    );
    assert_eq!(commit_count(dir.path()), before + 2);
    assert_eq!(
        last_subject(dir.path()),
        format!("waap agent update {agent_id}")
    );
}

#[test]
fn agent_update_rejects_invalid_lifecycle_transition_without_commit() {
    let dir = tempdir().unwrap();
    init_repo_with_waap_project(dir.path());
    let output = waap(
        dir.path(),
        "# Purpose\n",
        &["--output-format", "json", "agent", "new"],
    );
    assert!(output.status.success());
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let agent_id = value["agent_id"].as_str().unwrap();
    let before = commit_count(dir.path());

    let output = waap(
        dir.path(),
        "",
        &[
            "agent",
            "update",
            "--agent-id",
            agent_id,
            "--set-status",
            "completed",
        ],
    );

    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr)
        .contains("invalid agent status transition: ready -> completed"));
    assert_eq!(commit_count(dir.path()), before);
}

#[test]
fn agent_update_rejects_aborting_running_agent_without_change_or_commit() {
    let dir = tempdir().unwrap();
    init_repo_with_waap_project(dir.path());
    let output = waap(
        dir.path(),
        "# Purpose\n",
        &["--output-format", "json", "agent", "new"],
    );
    assert!(output.status.success());
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let agent_id = value["agent_id"].as_str().unwrap();
    let output = waap(
        dir.path(),
        "",
        &[
            "agent",
            "update",
            "--agent-id",
            agent_id,
            "--set-status",
            "running",
        ],
    );
    assert!(output.status.success());
    let before_commit = git(&state_root(dir.path()), &["rev-parse", "HEAD"]);
    let agent_path = state_root(dir.path()).join(format!("agents/{agent_id}/agent.md"));
    let before_record = std::fs::read_to_string(&agent_path).unwrap();

    let output = waap(
        dir.path(),
        "",
        &[
            "agent",
            "update",
            "--agent-id",
            agent_id,
            "--set-status",
            "aborted",
        ],
    );

    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr)
        .contains(&format!("waap agent stop --agent-id {agent_id}")));
    assert_eq!(std::fs::read_to_string(agent_path).unwrap(), before_record);
    assert_eq!(
        git(&state_root(dir.path()), &["rev-parse", "HEAD"]),
        before_commit
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
    assert!(state_root(dir.path())
        .join("agents/aa-custom-agent123/agent.md")
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
    assert!(files.contains("tickets/tt-task/ticket.md"));
    assert!(!files.contains("user.txt"));
    // The user's staged change survives.
    assert!(git(dir.path(), &["diff", "--cached", "--name-only"]).contains("user.txt"));
}

#[test]
fn failed_commit_returns_error_but_keeps_state() {
    // Force the state worktree's index to be locked.
    let dir = tempdir().unwrap();
    init_repo_with_waap_project(dir.path());
    let git_file = std::fs::read_to_string(state_root(dir.path()).join(".git")).unwrap();
    let git_dir = state_root(dir.path()).join(git_file.trim_start_matches("gitdir: ").trim());
    std::fs::File::create(git_dir.join("index.lock")).unwrap();

    let output = waap(dir.path(), "# Body\n", &["ticket", "new", "--name", "Task"]);

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.starts_with("failed to create ticket: failed to commit waap state change:"),
        "{stderr}"
    );
    assert!(output.stdout.is_empty());
    // State update is intact on disk despite the commit failure.
    assert!(state_root(dir.path())
        .join("tickets/tt-task/ticket.md")
        .is_file());
}

#[test]
fn respects_waap_root_run_from_elsewhere() {
    let dir = tempdir().unwrap();
    let waap_root = dir.path().join("project");
    std::fs::create_dir_all(&waap_root).unwrap();
    init_repo_with_waap_project(&waap_root);

    // An explicit state directory is used directly.
    use std::io::Write;
    let explicit_state = state_root(&waap_root);
    let mut command = Command::new(env!("CARGO_BIN_EXE_waap"));
    isolate_git_config(&mut command);
    let mut child = command
        .current_dir(&waap_root)
        .args(["--waap-root", explicit_state.to_str().unwrap()])
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
    assert!(last_commit_files(&waap_root).contains("tickets/tt-task/ticket.md"));
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
fn agent_stop_commits_and_reports_the_commit() {
    let dir = tempdir().unwrap();
    init_repo_with_waap_project(dir.path());

    let output = waap(
        dir.path(),
        "# Purpose\n",
        &["--output-format", "json", "agent", "new"],
    );
    assert!(output.status.success());
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let agent_id = value["agent_id"].as_str().unwrap().to_string();

    let output = waap(
        dir.path(),
        "",
        &[
            "agent",
            "update",
            "--agent-id",
            &agent_id,
            "--set-status",
            "running",
        ],
    );
    assert!(output.status.success());

    let before = commit_count(dir.path());
    let output = waap(
        dir.path(),
        "",
        &[
            "--output-format",
            "json",
            "agent",
            "stop",
            "--agent-id",
            &agent_id,
        ],
    );

    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(
        value["commit"],
        git(&state_root(dir.path()), &["rev-parse", "HEAD"])
    );
    assert_eq!(value["stopped_agents"][0]["agent_id"], agent_id);
    assert_eq!(value["stopped_agents"][0]["metadata"]["status"], "aborted");
    assert_eq!(commit_count(dir.path()), before + 1);
    assert_eq!(
        last_subject(dir.path()),
        format!("waap agent stop {agent_id}")
    );
}

#[test]
fn init_creates_and_commits_waap_skeleton() {
    let dir = tempdir().unwrap();
    init_repo(dir.path());

    let application_head = git(dir.path(), &["rev-parse", "HEAD"]);
    let output = waap(dir.path(), "", &["--output-format", "json", "init"]);

    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    let value: serde_json::Value = serde_json::from_str(stdout.trim()).unwrap();
    let commit = value["commit"].as_str().unwrap();
    assert_eq!(commit, git(&state_root(dir.path()), &["rev-parse", "HEAD"]));

    assert_eq!(commit_count(dir.path()), 1);
    assert_eq!(last_subject(dir.path()), "waap init");
    assert_eq!(git(dir.path(), &["rev-parse", "HEAD"]), application_head);
    assert!(state_root(dir.path()).join("agents").is_dir());
    assert!(state_root(dir.path()).join("tickets").is_dir());
}

#[test]
fn init_errors_when_waap_already_exists() {
    let dir = tempdir().unwrap();
    init_repo_with_waap_project(dir.path());

    let output = waap(dir.path(), "", &["init"]);

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains(".state"), "{stderr}");
}

#[test]
fn init_errors_outside_git_repository() {
    let dir = tempdir().unwrap();

    let output = waap(dir.path(), "", &["init"]);

    assert!(!output.status.success());
    assert!(!state_root(dir.path()).exists());
}

#[test]
fn ticket_new_errors_when_project_not_initialized() {
    let dir = tempdir().unwrap();
    init_repo(dir.path());

    let output = waap(dir.path(), "# Body\n", &["ticket", "new", "--name", "Task"]);

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("waap init"), "{stderr}");
    assert!(!state_root(dir.path()).exists());
}

#[test]
fn agent_new_errors_when_project_not_initialized() {
    let dir = tempdir().unwrap();
    init_repo(dir.path());

    let output = waap(dir.path(), "# Purpose\n", &["agent", "new"]);

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("waap init"), "{stderr}");
    assert!(!state_root(dir.path()).exists());
}

#[test]
fn dependency_workflow_preserves_body_and_releases_completed_dependencies() {
    let dir = tempdir().unwrap();
    init_repo_with_waap_project(dir.path());
    assert!(waap(dir.path(), "", &["ticket", "new", "--name", "Base"])
        .status
        .success());
    assert!(waap(
        dir.path(),
        "# Body\r\n+++\r\nEnd",
        &[
            "ticket",
            "new",
            "--name",
            "Feature",
            "--depends-on",
            "tt-base",
        ]
    )
    .status
    .success());
    let list = |filter| {
        let output = waap(
            dir.path(),
            "",
            &["--output-format", "json", "ticket", "list", filter],
        );
        assert!(output.status.success());
        serde_json::from_slice::<serde_json::Value>(&output.stdout).unwrap()
    };
    assert_eq!(list("--blocked")[0]["ticket_id"], "tt-feature");
    assert!(waap(
        dir.path(),
        "",
        &[
            "ticket",
            "update",
            "--ticket-id",
            "tt-base",
            "--set-status",
            "completed",
        ]
    )
    .status
    .success());
    assert_eq!(list("--unblocked").as_array().unwrap().len(), 2);
    let output = waap(
        dir.path(),
        "",
        &[
            "--output-format",
            "json",
            "ticket",
            "get",
            "--ticket-id",
            "tt-feature",
        ],
    );
    assert!(output.status.success());
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value["content"], "# Body\r\n+++\r\nEnd");
    assert!(waap(dir.path(), "", &["check"]).status.success());
}

#[test]
fn cyclic_update_is_rejected_without_changing_record_or_commit() {
    let dir = tempdir().unwrap();
    init_repo_with_waap_project(dir.path());
    assert!(
        waap(dir.path(), "base", &["ticket", "new", "--name", "Base"])
            .status
            .success()
    );
    assert!(waap(
        dir.path(),
        "feature",
        &[
            "ticket",
            "new",
            "--name",
            "Feature",
            "--depends-on",
            "tt-base",
        ]
    )
    .status
    .success());
    let path = state_root(dir.path()).join("tickets/tt-base/ticket.md");
    let record = std::fs::read(&path).unwrap();
    let before = commit_count(dir.path());
    for dependency in ["tt-base", "tt-feature"] {
        let output = waap(
            dir.path(),
            "",
            &[
                "ticket",
                "update",
                "--ticket-id",
                "tt-base",
                "--add-depends-on",
                dependency,
            ],
        );
        assert!(!output.status.success());
        assert!(String::from_utf8_lossy(&output.stderr).contains("dependency cycle detected"));
        assert_eq!(std::fs::read(&path).unwrap(), record);
        assert_eq!(commit_count(dir.path()), before);
        assert!(waap(dir.path(), "", &["check"]).status.success());
    }
}

#[test]
fn concurrent_named_creations_preserve_every_record_and_commit() {
    let dir = tempdir().unwrap();
    init_repo_with_waap_project(dir.path());
    let barrier = std::sync::Barrier::new(8);
    let before = commit_count(dir.path());
    std::thread::scope(|scope| {
        let handles: Vec<_> = (0..8)
            .map(|index| {
                let barrier = &barrier;
                let root = dir.path();
                scope.spawn(move || {
                    barrier.wait();
                    let output = waap(
                        root,
                        &format!("body {index}"),
                        &[
                            "--output-format",
                            "json",
                            "ticket",
                            "new",
                            "--name",
                            "Shared",
                        ],
                    );
                    assert!(
                        output.status.success(),
                        "{}",
                        String::from_utf8_lossy(&output.stderr)
                    );
                    let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
                    value["ticket_id"].as_str().unwrap().to_string()
                })
            })
            .collect();
        let ids: std::collections::HashSet<_> =
            handles.into_iter().map(|h| h.join().unwrap()).collect();
        assert_eq!(ids.len(), 8);
    });
    assert_eq!(commit_count(dir.path()), before + 8);
    assert!(waap(dir.path(), "", &["check"]).status.success());
    assert_eq!(git(&state_root(dir.path()), &["status", "--porcelain"]), "");
}

#[test]
fn concurrent_opposite_dependencies_cannot_publish_a_cycle() {
    let dir = tempdir().unwrap();
    init_repo_with_waap_project(dir.path());
    for name in ["A", "B"] {
        assert!(waap(dir.path(), "", &["ticket", "new", "--name", name])
            .status
            .success());
    }
    let before = commit_count(dir.path());
    let barrier = std::sync::Barrier::new(2);
    let successes = std::thread::scope(|scope| {
        let handles: Vec<_> = [("tt-a", "tt-b"), ("tt-b", "tt-a")]
            .into_iter()
            .map(|(id, dep)| {
                let root = dir.path();
                let barrier = &barrier;
                scope.spawn(move || {
                    barrier.wait();
                    waap(
                        root,
                        "",
                        &[
                            "ticket",
                            "update",
                            "--ticket-id",
                            id,
                            "--add-depends-on",
                            dep,
                        ],
                    )
                    .status
                    .success()
                })
            })
            .collect();
        handles
            .into_iter()
            .map(|handle| usize::from(handle.join().unwrap()))
            .sum::<usize>()
    });
    assert_eq!(successes, 1);
    assert_eq!(commit_count(dir.path()), before + 1);
    assert!(waap(dir.path(), "", &["check"]).status.success());
}
