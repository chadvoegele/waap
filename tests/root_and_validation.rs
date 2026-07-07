//! End-to-end tests for root resolution and command validation.

mod common;

use std::path::Path;
use std::process::{Command, Output, Stdio};

use tempfile::tempdir;

use common::{init_repo, isolate_git_config};

fn waap(cwd: &Path, stdin: &str, args: &[&str]) -> Output {
    use std::io::Write;

    let mut command = Command::new(env!("CARGO_BIN_EXE_waap"));
    isolate_git_config(&mut command);
    let mut child = command
        .current_dir(cwd)
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

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

#[test]
fn init_from_subdirectory_uses_git_root() {
    let dir = tempdir().unwrap();
    init_repo(dir.path());
    let sub = dir.path().join("deep/nested");
    std::fs::create_dir_all(&sub).unwrap();

    let output = waap(&sub, "", &["init"]);

    assert!(output.status.success(), "{}", stdout(&output));
    assert!(dir.path().join(".waap").is_dir());
    assert!(!sub.join(".waap").exists());
}

#[test]
fn init_with_explicit_root_uses_that_directory() {
    let dir = tempdir().unwrap();
    init_repo(dir.path());
    let project = dir.path().join("nested-project");
    std::fs::create_dir_all(&project).unwrap();

    let output = waap(
        dir.path(),
        "",
        &["--waap-root", project.to_str().unwrap(), "init"],
    );

    assert!(output.status.success(), "{}", stdout(&output));
    assert!(project.join(".waap").is_dir());
    assert!(!dir.path().join(".waap").exists());
}

#[test]
fn check_fails_when_waap_is_missing() {
    let dir = tempdir().unwrap();
    init_repo(dir.path());

    let output = waap(dir.path(), "", &["check"]);

    assert!(!output.status.success());
    assert!(stdout(&output).contains("no waap project found; run 'waap init'"));
}

#[test]
fn agent_and_ticket_commands_do_not_initialize_missing_state() {
    let dir = tempdir().unwrap();
    init_repo(dir.path());

    let agent = waap(dir.path(), "# Agent\n", &["agent", "new"]);
    let ticket = waap(
        dir.path(),
        "# Ticket\n",
        &["ticket", "new", "--name", "Task"],
    );

    assert!(!agent.status.success());
    assert!(!ticket.status.success());
    assert!(stderr(&agent).contains("run 'waap init'"));
    assert!(stderr(&ticket).contains("run 'waap init'"));
    assert!(!dir.path().join(".waap").exists());
}

#[test]
fn agent_and_ticket_commands_do_not_operate_on_invalid_state() {
    let dir = tempdir().unwrap();
    init_repo(dir.path());
    assert!(waap(dir.path(), "", &["init"]).status.success());
    std::fs::create_dir_all(dir.path().join(".waap/agents/invalid-agent")).unwrap();

    let agent = waap(dir.path(), "# Agent\n", &["agent", "new"]);
    let ticket = waap(
        dir.path(),
        "# Ticket\n",
        &["ticket", "new", "--name", "Task"],
    );

    assert!(!agent.status.success());
    assert!(!ticket.status.success());
    assert!(stderr(&agent).contains("must be named as an agent id"));
    assert!(stderr(&ticket).contains("must be named as an agent id"));
    assert_eq!(
        std::fs::read_dir(dir.path().join(".waap/agents"))
            .unwrap()
            .count(),
        1
    );
    assert_eq!(
        std::fs::read_dir(dir.path().join(".waap/tickets"))
            .unwrap()
            .count(),
        0
    );
}
