//! End-to-end tests for central `waap check`.

mod common;

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
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

fn state_root(parent: &Path) -> PathBuf {
    parent.join("state")
}

fn derived_state_root(home: &Path, repository: &Path) -> PathBuf {
    home.join(".local/state/waap/data").join(
        repository
            .canonicalize()
            .unwrap()
            .strip_prefix("/")
            .unwrap(),
    )
}

fn init_state(repository: &Path, state: &Path) {
    let output = waap(
        repository,
        &["--waap-root", state.to_str().unwrap(), "init"],
        None,
    );
    assert!(output.status.success(), "{}", stderr(&output));
}

fn bare_remote(parent: &Path) -> PathBuf {
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
fn check_reports_a_shared_derived_state_directory_from_all_worktrees() {
    let repository = tempdir().unwrap();
    let home = tempdir().unwrap();
    init_repo(repository.path());
    assert!(waap(repository.path(), &["init"], Some(home.path()))
        .status
        .success());
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

    let reports = [repository.path(), first.as_path(), second.as_path()].map(|cwd| {
        let output = waap(
            cwd,
            &["--output-format", "json", "check"],
            Some(home.path()),
        );
        assert!(output.status.success(), "{}", stderr(&output));
        serde_json::from_slice::<serde_json::Value>(&output.stdout).unwrap()
    });

    assert!(reports.iter().all(|report| report["valid"] == true));
    assert_eq!(reports[0]["state_directory"], reports[1]["state_directory"]);
    assert_eq!(reports[0]["state_directory"], reports[2]["state_directory"]);
}

#[test]
fn check_reports_absent_legacy_and_coexisting_state_directories() {
    let repository = tempdir().unwrap();
    let home = tempdir().unwrap();
    let state = derived_state_root(home.path(), repository.path());
    init_repo(repository.path());

    let absent = waap(
        repository.path(),
        &["--output-format", "json", "check"],
        Some(home.path()),
    );
    assert!(!absent.status.success());
    let absent: serde_json::Value = serde_json::from_slice(&absent.stdout).unwrap();
    assert_eq!(absent["state_directory"], state.display().to_string());

    fs::create_dir(repository.path().join(".waap")).unwrap();
    let legacy = waap(
        repository.path(),
        &["--output-format", "json", "check"],
        Some(home.path()),
    );
    assert!(!legacy.status.success());
    let legacy: serde_json::Value = serde_json::from_slice(&legacy.stdout).unwrap();
    assert!(legacy["errors"].to_string().contains("requires migration"));
    assert!(legacy["state_directory"].as_str().unwrap().starts_with('/'));

    fs::remove_dir(repository.path().join(".waap")).unwrap();
    assert!(waap(repository.path(), &["init"], Some(home.path()))
        .status
        .success());
    fs::create_dir(repository.path().join(".waap")).unwrap();
    let coexistence = waap(
        repository.path(),
        &[
            "--waap-root",
            state.to_str().unwrap(),
            "--output-format",
            "json",
            "check",
        ],
        None,
    );
    assert!(coexistence.status.success(), "{}", stderr(&coexistence));
    assert!(
        serde_json::from_slice::<serde_json::Value>(&coexistence.stdout).unwrap()["valid"] == true
    );

    let coexistence = waap(
        repository.path(),
        &["--output-format", "json", "check"],
        Some(home.path()),
    );
    assert!(!coexistence.status.success());
    let coexistence: serde_json::Value = serde_json::from_slice(&coexistence.stdout).unwrap();
    let errors = coexistence["errors"].to_string();
    assert!(errors.contains("coexist"));
    assert!(errors.contains(".waap"));
    assert!(errors.contains(&state.display().to_string()));
}

#[test]
fn check_rejects_bad_upstream_history_and_worktree_registration() {
    let repository = tempdir().unwrap();
    let state_parent = tempdir().unwrap();
    let remote_parent = tempdir().unwrap();
    let state = state_root(state_parent.path());
    init_repo(repository.path());
    let remote = bare_remote(remote_parent.path());
    git(
        repository.path(),
        &["remote", "add", "origin", remote.to_str().unwrap()],
    );
    init_state(repository.path(), &state);

    git(
        repository.path(),
        &["config", "--unset", "branch.waap.merge"],
    );
    let upstream = waap(
        repository.path(),
        &[
            "--waap-root",
            state.to_str().unwrap(),
            "--output-format",
            "json",
            "check",
        ],
        None,
    );
    assert!(!upstream.status.success());
    assert!(
        serde_json::from_slice::<serde_json::Value>(&upstream.stdout).unwrap()["errors"]
            .to_string()
            .contains("must track origin/waap")
    );
    git(
        repository.path(),
        &["config", "branch.waap.merge", "refs/heads/waap"],
    );

    fs::write(state.join("not-state.txt"), "bad history\n").unwrap();
    git(&state, &["add", "not-state.txt"]);
    git(&state, &["commit", "-q", "-m", "bad state history"]);
    let history = waap(
        repository.path(),
        &[
            "--waap-root",
            state.to_str().unwrap(),
            "--output-format",
            "json",
            "check",
        ],
        None,
    );
    assert!(!history.status.success());
    assert!(
        serde_json::from_slice::<serde_json::Value>(&history.stdout).unwrap()["errors"]
            .to_string()
            .contains("non-state path not-state.txt")
    );

    let moved = state_parent.path().join("moved-state");
    git(
        repository.path(),
        &[
            "worktree",
            "move",
            state.to_str().unwrap(),
            moved.to_str().unwrap(),
        ],
    );
    let registration = waap(
        repository.path(),
        &[
            "--waap-root",
            state.to_str().unwrap(),
            "--output-format",
            "json",
            "check",
        ],
        None,
    );
    assert!(!registration.status.success());
    assert!(
        serde_json::from_slice::<serde_json::Value>(&registration.stdout).unwrap()["errors"]
            .to_string()
            .contains("registered at")
    );
}

#[test]
fn check_rejects_all_direct_state_changes_and_keeps_json_on_stdout() {
    let repository = tempdir().unwrap();
    let state_parent = tempdir().unwrap();
    let state = state_root(state_parent.path());
    init_repo(repository.path());
    init_state(repository.path(), &state);

    fs::write(state.join("agents/.gitkeep"), "staged\n").unwrap();
    git(&state, &["add", "agents/.gitkeep"]);
    fs::write(state.join("tickets/.gitkeep"), "unstaged\n").unwrap();
    fs::write(state.join("untracked.md"), "untracked\n").unwrap();

    let output = waap(
        repository.path(),
        &[
            "--waap-root",
            state.to_str().unwrap(),
            "--output-format",
            "json",
            "check",
        ],
        None,
    );
    assert!(!output.status.success());
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["valid"], false);
    let errors = report["errors"].to_string();
    assert!(errors.contains("agents/.gitkeep"));
    assert!(errors.contains("tickets/.gitkeep"));
    assert!(errors.contains("untracked.md"));
    assert!(output.stderr.is_empty());
}

#[test]
fn check_uses_cached_remote_state_without_network_io() {
    let repository = tempdir().unwrap();
    let state_parent = tempdir().unwrap();
    let remote_parent = tempdir().unwrap();
    let source_parent = tempdir().unwrap();
    let state = state_root(state_parent.path());
    init_repo(repository.path());
    let remote = bare_remote(remote_parent.path());
    git(
        repository.path(),
        &["remote", "add", "origin", remote.to_str().unwrap()],
    );
    init_state(repository.path(), &state);
    git(&state, &["push", "-q", "origin", "waap"]);

    let source = source_parent.path().join("source");
    let mut clone = Command::new("git");
    isolate_git_config(&mut clone);
    assert!(clone
        .args([
            "clone",
            "-q",
            remote.to_str().unwrap(),
            source.to_str().unwrap()
        ])
        .status()
        .unwrap()
        .success());
    git(&source, &["switch", "-q", "waap"]);
    fs::write(source.join("agents/.gitkeep"), "remote-only\n").unwrap();
    git(&source, &["add", "agents/.gitkeep"]);
    git(&source, &["commit", "-q", "-m", "remote state"]);
    git(&source, &["push", "-q", "origin", "waap"]);
    git(repository.path(), &["fetch", "-q", "origin", "waap"]);

    let remote_only = waap(
        repository.path(),
        &[
            "--waap-root",
            state.to_str().unwrap(),
            "--output-format",
            "json",
            "check",
        ],
        None,
    );
    assert!(remote_only.status.success(), "{}", stderr(&remote_only));
    let report: serde_json::Value = serde_json::from_slice(&remote_only.stdout).unwrap();
    assert_eq!(report["valid"], true);
    assert!(report["warnings"]
        .to_string()
        .contains("commit(s) not in local waap"));
    assert!(stderr(&remote_only).contains("commit(s) not in local waap"));

    git(
        repository.path(),
        &["update-ref", "-d", "refs/remotes/origin/waap"],
    );
    let network_marker = remote_parent.path().join("network-invoked");
    let ssh = remote_parent.path().join("ssh");
    fs::write(
        &ssh,
        format!("#!/bin/sh\ntouch {}\nexit 1\n", network_marker.display()),
    )
    .unwrap();
    fs::set_permissions(&ssh, fs::Permissions::from_mode(0o755)).unwrap();
    git(
        repository.path(),
        &["config", "core.sshCommand", ssh.to_str().unwrap()],
    );
    git(
        repository.path(),
        &["remote", "set-url", "origin", "ssh://example.invalid/waap"],
    );
    let unavailable = waap(
        repository.path(),
        &[
            "--waap-root",
            state.to_str().unwrap(),
            "--output-format",
            "json",
            "check",
        ],
        None,
    );
    assert!(unavailable.status.success(), "{}", stderr(&unavailable));
    let report: serde_json::Value = serde_json::from_slice(&unavailable.stdout).unwrap();
    assert_eq!(report["valid"], true);
    assert_eq!(report["warnings"], serde_json::json!([]));
    assert!(unavailable.stderr.is_empty());
    assert!(!network_marker.exists(), "waap check accessed the network");
}
