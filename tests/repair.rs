mod common;

use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

use tempfile::tempdir;

use common::{git, init_repo, isolate_git_config};

fn state_root(home: &Path, repository: &Path) -> PathBuf {
    home.join(".local/state/waap/data")
        .join(repository.strip_prefix("/").unwrap())
}

fn waap(home: &Path, cwd: &Path, args: &[&str]) -> Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_waap"));
    isolate_git_config(&mut command);
    command
        .env("HOME", home)
        .current_dir(cwd)
        .args(args)
        .stdin(Stdio::null())
        .output()
        .unwrap()
}

#[test]
fn repair_relocates_state_after_repository_move() {
    let dir = tempdir().unwrap();
    let home = dir.path().join("home");
    let old_repository = dir.path().join("old/repository");
    let new_repository = dir.path().join("new/repository");
    std::fs::create_dir_all(&old_repository).unwrap();
    init_repo(&old_repository);
    assert!(waap(&home, &old_repository, &["init"]).status.success());

    let old_state = state_root(&home, &old_repository);
    std::fs::write(old_state.join("preserved.txt"), "state\n").unwrap();
    git(&old_state, &["add", "preserved.txt"]);
    git(&old_state, &["commit", "-q", "-m", "preserve state"]);
    let state_head = git(&old_state, &["rev-parse", "HEAD"]);
    let application_head = git(&old_repository, &["rev-parse", "HEAD"]);

    std::fs::create_dir_all(new_repository.parent().unwrap()).unwrap();
    std::fs::rename(&old_repository, &new_repository).unwrap();
    let output = waap(&home, &new_repository, &["repair"]);

    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let new_state = state_root(&home, &new_repository);
    assert!(!old_state.exists());
    assert_eq!(
        std::fs::read_to_string(new_state.join("preserved.txt")).unwrap(),
        "state\n"
    );
    assert_eq!(git(&new_state, &["branch", "--show-current"]), "waap");
    assert_eq!(git(&new_state, &["rev-parse", "HEAD"]), state_head);
    assert_eq!(
        git(&new_repository, &["rev-parse", "HEAD"]),
        application_head
    );
    assert!(String::from_utf8_lossy(&output.stdout).contains("Relocated from:"));
}

#[test]
fn repair_is_idempotent() {
    let dir = tempdir().unwrap();
    let home = dir.path().join("home");
    let repository = dir.path().join("repository");
    std::fs::create_dir_all(&repository).unwrap();
    init_repo(&repository);
    assert!(waap(&home, &repository, &["init"]).status.success());

    let output = waap(&home, &repository, &["repair"]);

    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(String::from_utf8_lossy(&output.stdout).contains("already repaired"));
}

#[test]
fn repair_refuses_an_occupied_destination() {
    let dir = tempdir().unwrap();
    let home = dir.path().join("home");
    let old_repository = dir.path().join("old/repository");
    let new_repository = dir.path().join("new/repository");
    std::fs::create_dir_all(&old_repository).unwrap();
    init_repo(&old_repository);
    assert!(waap(&home, &old_repository, &["init"]).status.success());
    let old_state = state_root(&home, &old_repository);

    std::fs::create_dir_all(new_repository.parent().unwrap()).unwrap();
    std::fs::rename(&old_repository, &new_repository).unwrap();
    let new_state = state_root(&home, &new_repository);
    std::fs::create_dir_all(&new_state).unwrap();

    let output = waap(&home, &new_repository, &["repair"]);

    assert!(!output.status.success());
    assert!(old_state.exists());
    assert!(String::from_utf8_lossy(&output.stderr).contains("already exists"));
}
