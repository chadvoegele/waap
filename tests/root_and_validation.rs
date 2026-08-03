//! End-to-end tests for central `waap init` setup.

mod common;

use std::fs;
use std::path::Path;
use std::process::{Command, Output, Stdio};

use tempfile::tempdir;

use common::{git, init_repo, isolate_git_config};

fn waap(cwd: &Path, args: &[&str], home: Option<&Path>) -> Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_waap"));
    isolate_git_config(&mut command);
    command.env_remove("WAAP_LOG_LEVEL");
    if let Some(home) = home {
        command.env("HOME", home);
    }
    command
        .current_dir(cwd)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .unwrap()
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

fn derived_state_directory(home: &Path, repository: &Path) -> std::path::PathBuf {
    home.join(".local/state/waap/data").join(
        repository
            .canonicalize()
            .unwrap()
            .strip_prefix("/")
            .unwrap(),
    )
}

fn bare_remote(parent: &Path) -> std::path::PathBuf {
    let remote = parent.join("remote.git");
    let mut command = Command::new("git");
    isolate_git_config(&mut command);
    assert!(command
        .args(["init", "-q", "--bare", remote.to_str().unwrap()])
        .status()
        .unwrap()
        .success());
    remote
}

#[test]
fn fresh_init_uses_derived_state_and_leaves_application_unchanged() {
    let repository = tempdir().unwrap();
    let home = tempdir().unwrap();
    init_repo(repository.path());
    let application_head = git(repository.path(), &["rev-parse", "HEAD"]);
    let expected_state = derived_state_directory(home.path(), repository.path());

    let output = waap(
        repository.path(),
        &["--output-format", "json", "init"],
        Some(home.path()),
    );

    assert!(output.status.success(), "{}", stderr(&output));
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(
        report["state_directory"],
        expected_state.display().to_string()
    );
    assert_eq!(
        report["commit"],
        git(&expected_state, &["rev-parse", "HEAD"])
    );
    assert!(expected_state.join("agents").is_dir());
    assert!(expected_state.join("tickets").is_dir());
    assert_eq!(
        git(repository.path(), &["rev-parse", "HEAD"]),
        application_head
    );
    assert!(git(repository.path(), &["status", "--porcelain"]).is_empty());
    assert!(!repository.path().join(".waap").exists());
}

#[test]
fn fresh_init_supports_an_unborn_application_repository() {
    let repository = tempdir().unwrap();
    let home = tempdir().unwrap();
    git(repository.path(), &["init", "-q", "--initial-branch=main"]);
    git(repository.path(), &["config", "user.name", "Test"]);
    git(
        repository.path(),
        &["config", "user.email", "test@example.com"],
    );
    let state = derived_state_directory(home.path(), repository.path());

    let output = waap(repository.path(), &["init"], Some(home.path()));

    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(
        git(repository.path(), &["symbolic-ref", "--short", "HEAD"]),
        "main"
    );
    assert!(git(repository.path(), &["status", "--porcelain"]).is_empty());
    let mut verify_head = Command::new("git");
    isolate_git_config(&mut verify_head);
    assert!(!verify_head
        .current_dir(repository.path())
        .args(["rev-parse", "--verify", "HEAD"])
        .output()
        .unwrap()
        .status
        .success());
    assert_eq!(git(&state, &["branch", "--show-current"]), "waap");
    assert_eq!(git(&state, &["log", "-1", "--pretty=%s"]), "waap init");
    assert!(state.join("agents/.gitkeep").is_file());
    assert!(state.join("tickets/.gitkeep").is_file());
}

#[test]
fn fresh_init_creates_a_parentless_state_history_and_pushes_its_upstream() {
    let repository = tempdir().unwrap();
    let remote_parent = tempdir().unwrap();
    let home = tempdir().unwrap();
    init_repo(repository.path());
    let remote = bare_remote(remote_parent.path());
    git(
        repository.path(),
        &["remote", "add", "origin", remote.to_str().unwrap()],
    );
    let application_head = git(repository.path(), &["rev-parse", "HEAD"]);
    let state = derived_state_directory(home.path(), repository.path());

    let output = waap(repository.path(), &["init"], Some(home.path()));

    assert!(output.status.success(), "{}", stderr(&output));
    assert!(!git(&state, &["rev-list", "--max-parents=0", "waap"]).is_empty());
    assert!(git(&state, &["rev-list", "--parents", "waap"])
        .lines()
        .all(|line| line.split_whitespace().count() == 1));
    assert!(git(&state, &["log", "--format=", "--name-only", "waap"])
        .lines()
        .filter(|path| !path.is_empty())
        .all(|path| path.starts_with("agents/") || path.starts_with("tickets/")));
    assert_eq!(
        git(repository.path(), &["config", "branch.waap.remote"]),
        "origin"
    );
    assert_eq!(
        git(repository.path(), &["config", "branch.waap.merge"]),
        "refs/heads/waap"
    );
    git(&state, &["push", "-q"]);
    assert_eq!(
        git(remote.as_path(), &["rev-parse", "refs/heads/waap"]),
        git(&state, &["rev-parse", "waap"])
    );
    assert_eq!(
        git(repository.path(), &["rev-parse", "HEAD"]),
        application_head
    );
}

#[test]
fn init_uses_an_exact_override_target() {
    let repository = tempdir().unwrap();
    let state_parent = tempdir().unwrap();
    let state = state_parent.path().join("exact-state");
    init_repo(repository.path());
    let application_head = git(repository.path(), &["rev-parse", "HEAD"]);

    let output = waap(
        repository.path(),
        &["--waap-root", state.to_str().unwrap(), "init"],
        None,
    );

    assert!(output.status.success(), "{}", stderr(&output));
    assert!(String::from_utf8_lossy(&output.stdout).contains(&format!(
        "State directory: {}",
        state.canonicalize().unwrap().display()
    )));
    assert_eq!(
        git(repository.path(), &["rev-parse", "HEAD"]),
        application_head
    );
    assert!(state.join("agents").is_dir());
    assert!(state.join("tickets").is_dir());
}

#[test]
fn init_adopts_verified_origin_state_without_changing_the_application() {
    let repository = tempdir().unwrap();
    let remote_parent = tempdir().unwrap();
    let source = tempdir().unwrap();
    let home = tempdir().unwrap();
    let remote = bare_remote(remote_parent.path());
    init_repo(repository.path());
    init_repo(source.path());
    git(source.path(), &["switch", "--orphan", "waap"]);
    fs::create_dir_all(source.path().join("agents/aa-one")).unwrap();
    fs::create_dir_all(source.path().join("tickets/tt-one")).unwrap();
    fs::write(source.path().join("agents/aa-one/agent.md"), "+++").unwrap();
    fs::write(source.path().join("tickets/tt-one/ticket.md"), "+++").unwrap();
    git(source.path(), &["add", "agents", "tickets"]);
    git(source.path(), &["commit", "-q", "-m", "remote state"]);
    let remote_head = git(source.path(), &["rev-parse", "HEAD"]);
    git(
        source.path(),
        &["remote", "add", "origin", remote.to_str().unwrap()],
    );
    git(source.path(), &["push", "-q", "origin", "waap"]);
    git(
        repository.path(),
        &["remote", "add", "origin", remote.to_str().unwrap()],
    );
    let application_head = git(repository.path(), &["rev-parse", "HEAD"]);
    let state = derived_state_directory(home.path(), repository.path());

    let output = waap(repository.path(), &["init"], Some(home.path()));

    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(git(&state, &["rev-parse", "HEAD"]), remote_head);
    assert!(state.join("agents/aa-one/agent.md").is_file());
    assert_eq!(
        git(
            repository.path(),
            &["config", "--get", "branch.waap.remote"]
        ),
        "origin"
    );
    assert_eq!(
        git(repository.path(), &["rev-parse", "HEAD"]),
        application_head
    );
}

#[test]
fn unreachable_origin_fails_before_creating_state() {
    let repository = tempdir().unwrap();
    let home = tempdir().unwrap();
    init_repo(repository.path());
    let state = derived_state_directory(home.path(), repository.path());
    let application_head = git(repository.path(), &["rev-parse", "HEAD"]);
    git(
        repository.path(),
        &[
            "remote",
            "add",
            "origin",
            repository.path().join("missing-remote").to_str().unwrap(),
        ],
    );

    let output = waap(repository.path(), &["init"], Some(home.path()));

    assert!(!output.status.success());
    assert!(stderr(&output).contains("ls-remote"), "{}", stderr(&output));
    assert!(!state.exists());
    assert!(git(repository.path(), &["branch", "--list", "waap"]).is_empty());
    assert_eq!(
        git(repository.path(), &["rev-parse", "HEAD"]),
        application_head
    );
}

#[test]
fn init_rejects_existing_state_idempotently() {
    let repository = tempdir().unwrap();
    let state_parent = tempdir().unwrap();
    let state = state_parent.path().join("state");
    init_repo(repository.path());
    assert!(waap(
        repository.path(),
        &["--waap-root", state.to_str().unwrap(), "init"],
        None
    )
    .status
    .success());
    let state_head = git(&state, &["rev-parse", "HEAD"]);
    let application_head = git(repository.path(), &["rev-parse", "HEAD"]);

    let output = waap(
        repository.path(),
        &["--waap-root", state.to_str().unwrap(), "init"],
        None,
    );

    assert!(!output.status.success());
    assert!(stderr(&output).contains("setup-only"));
    assert_eq!(git(&state, &["rev-parse", "HEAD"]), state_head);
    assert_eq!(
        git(repository.path(), &["rev-parse", "HEAD"]),
        application_head
    );
}

#[test]
fn init_preserves_a_conflicting_application_waap_branch() {
    let repository = tempdir().unwrap();
    let home = tempdir().unwrap();
    init_repo(repository.path());
    let application_head = git(repository.path(), &["rev-parse", "HEAD"]);
    let state = derived_state_directory(home.path(), repository.path());
    git(repository.path(), &["branch", "waap"]);

    let output = waap(repository.path(), &["init"], Some(home.path()));

    assert!(!output.status.success());
    assert!(stderr(&output).contains("non-state path README.md"));
    assert_eq!(
        git(repository.path(), &["rev-parse", "waap"]),
        application_head
    );
    assert_eq!(
        git(repository.path(), &["rev-parse", "HEAD"]),
        application_head
    );
    assert!(!state.exists());
}

#[test]
fn init_rejects_legacy_state_without_an_override() {
    let repository = tempdir().unwrap();
    let home = tempdir().unwrap();
    init_repo(repository.path());
    fs::create_dir_all(repository.path().join(".waap/agents")).unwrap();
    let state = derived_state_directory(home.path(), repository.path());

    let output = waap(repository.path(), &["init"], Some(home.path()));

    assert!(!output.status.success());
    assert!(stderr(&output).contains("legacy state"));
    assert!(!state.exists());
}
