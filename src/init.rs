use std::ffi::OsString;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde_json::json;

use crate::cli::OutputFormat;
use crate::git::{commit_paths, ref_exists, run_git, Committed};

const STATE_BRANCH: &str = "waap";

#[derive(Debug)]
pub(crate) struct InitReport {
    pub(crate) path: PathBuf,
}

fn args(values: &[&str]) -> Vec<OsString> {
    values.iter().map(OsString::from).collect()
}

fn has_origin(repository_root: &Path) -> io::Result<bool> {
    let output = run_git(repository_root, &args(&["remote"]))?;
    Ok(String::from_utf8_lossy(&output.stdout)
        .lines()
        .any(|remote| remote == "origin"))
}

fn create_state_worktree(repository_root: &Path, state_root: &Path) -> io::Result<bool> {
    if state_root.exists() {
        return Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            format!("{} already exists", state_root.display()),
        ));
    }
    if let Some(parent) = state_root.parent() {
        fs::create_dir_all(parent)?;
    }

    let local_branch = ref_exists(repository_root, "refs/heads/waap")?;
    let remote_branch = ref_exists(repository_root, "refs/remotes/origin/waap")?;
    let state_path = state_root.as_os_str().to_os_string();
    if local_branch {
        run_git(
            repository_root,
            &[
                "worktree".into(),
                "add".into(),
                state_path,
                STATE_BRANCH.into(),
            ],
        )?;
    } else if remote_branch {
        run_git(
            repository_root,
            &[
                "worktree".into(),
                "add".into(),
                "--track".into(),
                "-b".into(),
                STATE_BRANCH.into(),
                state_path,
                "origin/waap".into(),
            ],
        )?;
    } else {
        run_git(
            repository_root,
            &[
                "worktree".into(),
                "add".into(),
                "--orphan".into(),
                "-b".into(),
                STATE_BRANCH.into(),
                state_path,
            ],
        )?;
    }

    if has_origin(repository_root)? {
        run_git(
            repository_root,
            &args(&["config", "branch.waap.remote", "origin"]),
        )?;
        run_git(
            repository_root,
            &args(&["config", "branch.waap.merge", "refs/heads/waap"]),
        )?;
    }

    Ok(!local_branch && !remote_branch)
}

pub(crate) fn init_project(
    repository_root: &Path,
    state_root: &Path,
) -> io::Result<Committed<InitReport>> {
    let fresh = create_state_worktree(repository_root, state_root)?;
    if fresh {
        fs::create_dir_all(state_root.join("agents"))?;
        fs::create_dir_all(state_root.join("tickets"))?;
        let agent_marker = state_root.join("agents/.gitkeep");
        let ticket_marker = state_root.join("tickets/.gitkeep");
        fs::write(&agent_marker, "")?;
        fs::write(&ticket_marker, "")?;
        commit_paths(
            state_root,
            &[agent_marker.as_path(), ticket_marker.as_path()],
            "waap init",
        )?;
    }

    if !state_root.join("agents").is_dir() || !state_root.join("tickets").is_dir() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("{} is missing agents or tickets", state_root.display()),
        ));
    }

    let path = state_root.canonicalize()?;
    let commit = run_git(&path, &args(&["rev-parse", "HEAD"]))?;
    Ok(Committed {
        value: InitReport { path },
        commit: String::from_utf8_lossy(&commit.stdout).trim().to_owned(),
    })
}

pub(crate) fn print_init_report(output_format: &OutputFormat, committed: &Committed<InitReport>) {
    let report = &committed.value;
    match output_format {
        OutputFormat::Json => println!(
            "{}",
            json!({
                "path": report.path.display().to_string(),
                "state_directory": report.path.display().to_string(),
                "commit": committed.commit,
            })
        ),
        OutputFormat::HumanReadable => {
            println!("Initialized waap project");
            println!("State directory: {}", report.path.display());
            println!("Commit: {}", committed.commit);
        }
    }
}
