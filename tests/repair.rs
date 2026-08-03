//! End-to-end tests for legacy-state repair and migration.

mod common;

use std::fs;
use std::path::Path;
use std::process::{Command, Output, Stdio};

use tempfile::tempdir;

use common::{git, init_repo, isolate_git_config};

fn waap(cwd: &Path, args: &[&str], home: &Path) -> Output {
    waap_with_env(cwd, args, home, None)
}

fn waap_with_env(
    cwd: &Path,
    args: &[&str],
    home: &Path,
    environment: Option<(&str, &str)>,
) -> Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_waap"));
    isolate_git_config(&mut command);
    command
        .env_remove("WAAP_LOG_LEVEL")
        .env("HOME", home)
        .current_dir(cwd)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if let Some((key, value)) = environment {
        command.env(key, value);
    }
    command.output().unwrap()
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

fn write_legacy_state(repository: &Path) {
    fs::create_dir_all(repository.join(".waap/agents/aa-0123abcd")).unwrap();
    fs::create_dir_all(repository.join(".waap/tickets/tt-migrate-state")).unwrap();
    fs::write(
        repository.join(".waap/agents/aa-0123abcd/agent.md"),
        "+++\ncreation_date = 2026-08-03T01:00:00Z\nstatus = \"ready\"\n+++\n\n# Purpose\n",
    )
    .unwrap();
    fs::write(
        repository.join(".waap/tickets/tt-migrate-state/ticket.md"),
        "+++\nname = \"Migrate state\"\ncreation_date = 2026-08-03T01:00:00Z\nstatus = \"pending\"\n+++\n\n# State\n",
    )
    .unwrap();
}

#[test]
fn repair_migrates_dirty_legacy_state_after_the_central_commit() {
    let repository = tempdir().unwrap();
    let home = tempdir().unwrap();
    init_repo(repository.path());
    write_legacy_state(repository.path());
    git(repository.path(), &["add", ".waap"]);
    git(repository.path(), &["commit", "-q", "-m", "legacy state"]);
    let legacy_head = git(repository.path(), &["rev-parse", "HEAD"]);
    fs::write(
        repository
            .path()
            .join(".waap/tickets/tt-migrate-state/notes.md"),
        "dirty legacy state is retained\n",
    )
    .unwrap();
    let state = derived_state_directory(home.path(), repository.path());

    let output = waap(
        repository.path(),
        &["--output-format", "json", "repair"],
        home.path(),
    );

    assert!(output.status.success(), "{}", stderr(&output));
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["state_directory"], state.display().to_string());
    assert_eq!(
        report["migration_commit"],
        git(&state, &["rev-parse", "HEAD"])
    );
    assert_eq!(
        git(&state, &["log", "-1", "--pretty=%s"]),
        "waap migrate state"
    );
    assert_eq!(
        git(repository.path(), &["log", "-1", "--pretty=%s"]),
        "Remove legacy waap state"
    );
    assert_eq!(
        git(repository.path(), &["rev-parse", "HEAD~1"]),
        legacy_head
    );
    assert!(!repository.path().join(".waap").exists());
    assert_eq!(
        fs::read_to_string(repository.path().join("README.md")).unwrap(),
        "seed\n"
    );
    assert_eq!(
        fs::read_to_string(state.join("tickets/tt-migrate-state/notes.md")).unwrap(),
        "dirty legacy state is retained\n"
    );
    assert!(git(&state, &["log", "--format=", "--name-only"])
        .lines()
        .filter(|path| !path.is_empty())
        .all(|path| path.starts_with("agents/") || path.starts_with("tickets/")));
}

#[test]
fn repair_rejects_unrelated_application_changes_without_touching_state() {
    let repository = tempdir().unwrap();
    let home = tempdir().unwrap();
    init_repo(repository.path());
    write_legacy_state(repository.path());
    fs::write(repository.path().join("README.md"), "unrelated change\n").unwrap();
    let state = derived_state_directory(home.path(), repository.path());

    let output = waap(repository.path(), &["repair"], home.path());

    assert!(!output.status.success());
    assert!(
        stderr(&output).contains("outside .waap"),
        "{}",
        stderr(&output)
    );
    assert!(repository.path().join(".waap").exists());
    assert!(!state.exists());
}

#[test]
fn repair_cleanup_failure_keeps_both_recoverable_copies_and_later_reports_conflict() {
    let repository = tempdir().unwrap();
    let home = tempdir().unwrap();
    init_repo(repository.path());
    write_legacy_state(repository.path());
    let state = derived_state_directory(home.path(), repository.path());

    let failed = waap_with_env(
        repository.path(),
        &["repair"],
        home.path(),
        Some(("WAAP_REPAIR_FAIL_SOURCE_CLEANUP", "1")),
    );

    assert!(!failed.status.success());
    assert!(stderr(&failed).contains("injected source cleanup failure"));
    assert!(repository.path().join(".waap").is_dir());
    assert!(state.join("agents/aa-0123abcd/agent.md").is_file());
    assert_eq!(
        git(&state, &["log", "-1", "--pretty=%s"]),
        "waap migrate state"
    );

    let retry = waap(repository.path(), &["repair"], home.path());
    assert!(!retry.status.success());
    assert!(stderr(&retry).contains("coexist"), "{}", stderr(&retry));
}

#[test]
fn explicit_repair_uses_only_the_selected_state_directory() {
    let repository = tempdir().unwrap();
    let state_parent = tempdir().unwrap();
    let state = state_parent.path().join("state");
    let home = tempdir().unwrap();
    init_repo(repository.path());
    write_legacy_state(repository.path());
    assert!(waap(
        repository.path(),
        &["--waap-root", state.to_str().unwrap(), "init"],
        home.path()
    )
    .status
    .success());

    let output = waap(
        repository.path(),
        &["--waap-root", state.to_str().unwrap(), "repair"],
        home.path(),
    );

    assert!(output.status.success(), "{}", stderr(&output));
    assert!(repository.path().join(".waap").is_dir());
    assert_eq!(git(&state, &["log", "-1", "--pretty=%s"]), "waap init");
}
