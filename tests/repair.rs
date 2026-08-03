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

fn write_central_state(state: &Path) {
    fs::create_dir_all(state.join("agents/aa-0123abcd")).unwrap();
    fs::create_dir_all(state.join("tickets/tt-central-state")).unwrap();
    fs::write(
        state.join("agents/aa-0123abcd/agent.md"),
        "+++\ncreation_date = 2026-08-03T01:00:00Z\nstatus = \"ready\"\n+++\n\n# Purpose\n",
    )
    .unwrap();
    fs::write(
        state.join("tickets/tt-central-state/ticket.md"),
        "+++\nname = \"Central state\"\ncreation_date = 2026-08-03T01:00:00Z\nstatus = \"pending\"\n+++\n\n# State\n",
    )
    .unwrap();
    git(state, &["add", "agents", "tickets"]);
    git(state, &["commit", "-q", "-m", "state contents"]);
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
fn repair_relocates_a_moved_primary_repository_and_preserves_dirty_state() {
    let parent = tempdir().unwrap();
    let old_repository = parent.path().join("old-repository");
    let moved_repository = parent.path().join("moved-repository");
    let home = tempdir().unwrap();
    fs::create_dir(&old_repository).unwrap();
    init_repo(&old_repository);
    assert!(waap(&old_repository, &["init"], home.path())
        .status
        .success());
    let old_state = derived_state_directory(home.path(), &old_repository);
    write_central_state(&old_state);
    let state_head = git(&old_state, &["rev-parse", "HEAD"]);
    let application_head = git(&old_repository, &["rev-parse", "HEAD"]);

    fs::write(
        old_state.join("agents/aa-0123abcd/agent.md"),
        "+++\ncreation_date = 2026-08-03T01:00:00Z\nstatus = \"running\"\n+++\n\n# Purpose\n",
    )
    .unwrap();
    git(&old_state, &["add", "agents/aa-0123abcd/agent.md"]);
    fs::write(
        old_state.join("tickets/tt-central-state/ticket.md"),
        "+++\nname = \"Central state\"\ncreation_date = 2026-08-03T01:00:00Z\nstatus = \"completed\"\n+++\n\n# State\n",
    )
    .unwrap();
    fs::write(old_state.join("agents/aa-0123abcd/notes.md"), "untracked\n").unwrap();

    fs::rename(&old_repository, &moved_repository).unwrap();
    let expected_state = derived_state_directory(home.path(), &moved_repository);
    let check = waap(
        &moved_repository,
        &["--output-format", "json", "check"],
        home.path(),
    );
    assert!(!check.status.success());
    let check_report: serde_json::Value = serde_json::from_slice(&check.stdout).unwrap();
    assert_eq!(
        check_report["state_directory"],
        expected_state.display().to_string()
    );
    assert!(check_report["errors"]
        .to_string()
        .contains(&old_state.display().to_string()));

    let output = waap(
        &moved_repository,
        &["--output-format", "json", "repair"],
        home.path(),
    );

    assert!(output.status.success(), "{}", stderr(&output));
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(
        report["state_directory"],
        expected_state.display().to_string()
    );
    assert_eq!(report["relocated_from"], old_state.display().to_string());
    assert!(!old_state.exists());
    assert_eq!(git(&expected_state, &["rev-parse", "HEAD"]), state_head);
    assert_eq!(
        git(&moved_repository, &["rev-parse", "HEAD"]),
        application_head
    );
    assert_eq!(
        git(&expected_state, &["diff", "--cached", "--name-only"]),
        "agents/aa-0123abcd/agent.md"
    );
    assert_eq!(
        git(&expected_state, &["diff", "--name-only"]),
        "tickets/tt-central-state/ticket.md"
    );
    assert_eq!(
        git(
            &expected_state,
            &["ls-files", "--others", "--exclude-standard"]
        ),
        "agents/aa-0123abcd/notes.md"
    );
    assert!(git(&moved_repository, &["worktree", "list", "--porcelain"])
        .contains(&format!("worktree {}", expected_state.display())));
}

#[test]
fn repair_rejects_an_occupied_relocation_destination_without_changing_worktrees() {
    let parent = tempdir().unwrap();
    let old_repository = parent.path().join("old-repository");
    let moved_repository = parent.path().join("moved-repository");
    let home = tempdir().unwrap();
    fs::create_dir(&old_repository).unwrap();
    init_repo(&old_repository);
    assert!(waap(&old_repository, &["init"], home.path())
        .status
        .success());
    let old_state = derived_state_directory(home.path(), &old_repository);

    fs::rename(&old_repository, &moved_repository).unwrap();
    let expected_state = derived_state_directory(home.path(), &moved_repository);
    fs::create_dir_all(&expected_state).unwrap();
    fs::write(expected_state.join("occupied"), "do not move\n").unwrap();
    let output = waap(&moved_repository, &["repair"], home.path());

    assert!(!output.status.success());
    let diagnostic = stderr(&output);
    assert!(
        diagnostic.contains(&old_state.display().to_string()),
        "{diagnostic}"
    );
    assert!(
        diagnostic.contains(&expected_state.display().to_string()),
        "{diagnostic}"
    );
    assert!(diagnostic.contains("occupied"), "{diagnostic}");
    assert!(old_state.is_dir());
    assert_eq!(
        fs::read_to_string(expected_state.join("occupied")).unwrap(),
        "do not move\n"
    );
    assert!(git(&moved_repository, &["worktree", "list", "--porcelain"])
        .contains(&format!("worktree {}", old_state.display())));
}

#[test]
fn repair_configures_origin_waap_after_origin_is_added_or_upstream_is_wrong() {
    let repository = tempdir().unwrap();
    let home = tempdir().unwrap();
    let remote = tempdir().unwrap();
    init_repo(repository.path());
    assert!(waap(repository.path(), &["init"], home.path())
        .status
        .success());
    let state = derived_state_directory(home.path(), repository.path());

    let bare = remote.path().join("origin.git");
    let mut command = Command::new("git");
    isolate_git_config(&mut command);
    assert!(command
        .args(["init", "-q", "--bare", bare.to_str().unwrap()])
        .status()
        .unwrap()
        .success());
    git(
        repository.path(),
        &["remote", "add", "origin", bare.to_str().unwrap()],
    );
    git(
        repository.path(),
        &["config", "branch.waap.remote", "wrong-origin"],
    );
    git(
        repository.path(),
        &["config", "branch.waap.merge", "refs/heads/wrong"],
    );

    let output = waap(repository.path(), &["repair"], home.path());

    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(
        git(repository.path(), &["config", "branch.waap.remote"]),
        "origin"
    );
    assert_eq!(
        git(repository.path(), &["config", "branch.waap.merge"]),
        "refs/heads/waap"
    );
    assert!(state.is_dir());
}

#[test]
fn repair_from_a_broken_linked_worktree_requires_the_primary_checkout() {
    let parent = tempdir().unwrap();
    let old_repository = parent.path().join("old-repository");
    let moved_repository = parent.path().join("moved-repository");
    let linked_worktree = parent.path().join("linked-worktree");
    let home = tempdir().unwrap();
    fs::create_dir(&old_repository).unwrap();
    init_repo(&old_repository);
    git(
        &old_repository,
        &[
            "worktree",
            "add",
            linked_worktree.to_str().unwrap(),
            "-b",
            "linked",
        ],
    );
    assert!(waap(&old_repository, &["init"], home.path())
        .status
        .success());
    let old_state = derived_state_directory(home.path(), &old_repository);

    fs::rename(&old_repository, &moved_repository).unwrap();
    let output = waap(&linked_worktree, &["repair"], home.path());

    assert!(!output.status.success());
    assert!(
        stderr(&output).contains("run waap repair from the primary repository"),
        "{}",
        stderr(&output)
    );
    assert!(old_state.is_dir());
    assert!(!derived_state_directory(home.path(), &moved_repository).exists());
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
