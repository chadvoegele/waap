//! End-to-end tests for state writes and coordination independently of Git.

mod common;

use std::fs;
use std::io::{ErrorKind, Write};
use std::path::PathBuf;
use std::process::{Command, Output, Stdio};
use std::sync::Barrier;

use common::{git, init_repo, isolate_git_config};
use serde_json::Value;
use tempfile::{tempdir, TempDir};

struct Project {
    _temp: TempDir,
    repo: PathBuf,
    state: PathBuf,
}

impl Project {
    fn new() -> Self {
        let temp = tempdir().unwrap();
        let repo = temp.path().join("repo");
        fs::create_dir(&repo).unwrap();
        init_repo(&repo);
        let state = temp.path().join("state");
        Self {
            _temp: temp,
            repo,
            state,
        }
    }

    fn initialized() -> Self {
        let project = Self::new();
        project.json("", &["init"]);
        project
    }

    fn run(&self, input: &str, args: &[&str]) -> Output {
        let mut command = Command::new(env!("CARGO_BIN_EXE_waap"));
        isolate_git_config(&mut command);
        let mut child = command
            .current_dir(&self.repo)
            .args(["--waap-root", self.state.to_str().unwrap()])
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        if let Err(error) = child.stdin.take().unwrap().write_all(input.as_bytes()) {
            assert_eq!(error.kind(), ErrorKind::BrokenPipe);
        }
        child.wait_with_output().unwrap()
    }

    fn json(&self, input: &str, args: &[&str]) -> Value {
        let mut options = vec!["--output-format", "json"];
        options.extend_from_slice(args);
        let output = self.run(input, &options);
        assert!(output.status.success(), "{output:?}");
        let value: Value = serde_json::from_slice(&output.stdout).unwrap();
        assert!(value.get("commit").is_none(), "{value}");
        value
    }

    fn check(&self) {
        let output = self.run("", &["check"]);
        assert!(output.status.success(), "{output:?}");
    }
}

#[test]
fn init_and_mutations_work_outside_any_state_checkout() {
    let project = Project::new();
    let refs_before = git(&project.repo, &["show-ref"]);
    let worktrees_before = git(&project.repo, &["worktree", "list", "--porcelain"]);
    let report = project.json("", &["init"]);
    let state = project.state.canonicalize().unwrap();
    assert_eq!(report["state_directory"], state.to_str().unwrap());
    assert_eq!(report["path"], report["state_directory"]);
    assert!(state.join("agents").is_dir());
    assert!(state.join("tickets").is_dir());
    assert!(!state.join(".git").exists());
    assert_eq!(git(&project.repo, &["show-ref"]), refs_before);
    assert_eq!(
        git(&project.repo, &["worktree", "list", "--porcelain"]),
        worktrees_before
    );
    project.check();

    let name = "a\n+++\nb";
    for (kind, id_option, id, status) in [
        ("ticket", "--ticket-id", "tt-a-b", "completed"),
        ("agent", "--agent-id", "aa-a-b", "aborted"),
    ] {
        project.json("# Body\n", &[kind, "new", "--name", name]);
        project.json("", &[kind, "update", id_option, id, "--set-status", status]);
        let report = project.json("", &[kind, "get", id_option, id]);
        assert_eq!(report["metadata"]["name"], name);
        assert_eq!(report["metadata"]["status"], status);
        assert!(report["content"].as_str().unwrap().ends_with("# Body\n"));
    }
    project.json("", &["init"]);
    assert!(state.join("tickets/tt-a-b/ticket.md").exists());
    assert!(state.join("agents/aa-a-b/agent.md").exists());
    project.check();
}

#[test]
fn user_checkout_preserves_index_refs_and_git_config_while_index_is_locked() {
    let project = Project::new();
    let state = &project.state;
    fs::create_dir(state).unwrap();
    init_repo(state);
    fs::write(state.join("user.txt"), "staged user change\n").unwrap();
    git(state, &["add", "user.txt"]);
    git(state, &["remote", "add", "origin", "unused.git"]);

    let git_dir = state.join(".git");
    let lock = git_dir.join("index.lock");
    fs::write(&lock, "user lock").unwrap();
    let index = fs::read(git_dir.join("index")).unwrap();
    let config = fs::read(git_dir.join("config")).unwrap();
    let refs = git(state, &["show-ref"]);
    let head = fs::read(git_dir.join("HEAD")).unwrap();

    project.json("", &["init"]);
    project.json("# Body\n", &["ticket", "new", "--name", "My Task"]);
    project.json(
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
    project.json(
        "# Purpose\n",
        &["agent", "new", "--name", "Custom Agent_123"],
    );
    let report = project.json("", &["agent", "stop", "--agent-id", "aa-custom-agent123"]);
    assert_eq!(report["stopped_agents"][0]["metadata"]["status"], "aborted");
    assert_eq!(
        project.json("", &["agent", "stop"])["stopped_agents"],
        serde_json::json!([])
    );
    project.check();

    assert!(state.join("tickets/tt-my-task/ticket.md").is_file());
    assert_eq!(fs::read(&lock).unwrap(), b"user lock");
    assert_eq!(fs::read(git_dir.join("index")).unwrap(), index);
    assert_eq!(fs::read(git_dir.join("config")).unwrap(), config);
    assert_eq!(fs::read(git_dir.join("HEAD")).unwrap(), head);
    assert_eq!(git(state, &["show-ref"]), refs);
    assert_eq!(git(state, &["diff", "--cached", "--name-only"]), "user.txt");
    let untracked = git(state, &["ls-files", "--others", "--exclude-standard"]);
    assert!(untracked.contains("tickets/tt-my-task/ticket.md"));
    assert!(untracked.contains("agents/aa-custom-agent123/agent.md"));
    assert!(!untracked.contains(".waap-runtime"), "{untracked}");
}

#[test]
fn agent_update_rejects_invalid_transitions_without_changing_record() {
    let project = Project::initialized();
    let report = project.json("# Purpose\n", &["agent", "new"]);
    let id = report["agent_id"].as_str().unwrap();
    let path = project.state.join("agents").join(id).join("agent.md");
    let update = |status| {
        project.run(
            "",
            &["agent", "update", "--agent-id", id, "--set-status", status],
        )
    };
    let ready = fs::read(&path).unwrap();
    let rejected = update("completed");
    assert!(!rejected.status.success());
    assert!(String::from_utf8_lossy(&rejected.stderr)
        .contains("invalid agent status transition: ready -> completed"));
    assert_eq!(fs::read(&path).unwrap(), ready);
    assert!(update("running").status.success());
    let running = fs::read(&path).unwrap();
    let rejected = update("aborted");
    assert!(!rejected.status.success());
    assert!(String::from_utf8_lossy(&rejected.stderr)
        .contains(&format!("waap agent stop --agent-id {id}")));
    assert_eq!(fs::read(&path).unwrap(), running);
}

#[test]
fn dependency_workflow_preserves_body_and_releases_completed_dependencies() {
    let project = Project::initialized();
    project.json("", &["ticket", "new", "--name", "Base"]);
    let body = "# Body\r\n+++\r\nEnd";
    project.json(
        body,
        &[
            "ticket",
            "new",
            "--name",
            "Feature",
            "--depends-on",
            "tt-base",
        ],
    );
    let list = |filter| project.json("", &["ticket", "list", filter]);
    assert_eq!(list("--blocked")[0]["ticket_id"], "tt-feature");
    project.json(
        "",
        &[
            "ticket",
            "update",
            "--ticket-id",
            "tt-base",
            "--set-status",
            "completed",
        ],
    );
    assert_eq!(list("--unblocked").as_array().unwrap().len(), 2);
    assert_eq!(
        project.json("", &["ticket", "get", "--ticket-id", "tt-feature"])["content"],
        body
    );
    project.check();
}

#[test]
fn cyclic_update_is_rejected_without_changing_record() {
    let project = Project::initialized();
    project.json("base", &["ticket", "new", "--name", "Base"]);
    project.json(
        "feature",
        &[
            "ticket",
            "new",
            "--name",
            "Feature",
            "--depends-on",
            "tt-base",
        ],
    );
    let path = project.state.join("tickets/tt-base/ticket.md");
    let record = fs::read(&path).unwrap();
    for dependency in ["tt-base", "tt-feature"] {
        let output = project.run(
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
        assert_eq!(fs::read(&path).unwrap(), record);
        project.check();
    }
}

#[test]
fn concurrent_named_creations_preserve_every_record() {
    let project = &Project::initialized();
    let barrier = &Barrier::new(8);
    std::thread::scope(|scope| {
        let handles: Vec<_> = (0..8)
            .map(|index| {
                scope.spawn(move || {
                    let body = format!("body {index}");
                    barrier.wait();
                    let value = project.json(&body, &["ticket", "new", "--name", "Shared"]);
                    let id = value["ticket_id"].as_str().unwrap();
                    let contents = fs::read_to_string(
                        project.state.join("tickets").join(id).join("ticket.md"),
                    )
                    .unwrap();
                    assert!(contents.ends_with(&body), "{contents:?}");
                    id.to_owned()
                })
            })
            .collect();
        let ids: std::collections::HashSet<_> =
            handles.into_iter().map(|h| h.join().unwrap()).collect();
        assert_eq!(ids.len(), 8);
    });
    project.check();
}

#[test]
fn concurrent_opposite_dependencies_cannot_publish_a_cycle() {
    let project = &Project::initialized();
    for name in ["A", "B"] {
        project.json("", &["ticket", "new", "--name", name]);
    }
    let barrier = &Barrier::new(2);
    let successes = std::thread::scope(|scope| {
        let handles: Vec<_> = [("tt-a", "tt-b"), ("tt-b", "tt-a")]
            .into_iter()
            .map(|(id, dep)| {
                scope.spawn(move || {
                    barrier.wait();
                    project
                        .run(
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
            .map(|h| usize::from(h.join().unwrap()))
            .sum::<usize>()
    });
    assert_eq!(successes, 1);
    project.check();
}
