use std::ffi::OsString;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde_json::json;

use crate::cli::OutputFormat;
use crate::git::run_git;

const STATE_BRANCH_REF: &str = "refs/heads/waap";

#[derive(Debug)]
pub(crate) struct RepairReport {
    pub(crate) state_directory: PathBuf,
    pub(crate) relocated_from: Option<PathBuf>,
}

fn args(values: &[&str]) -> Vec<OsString> {
    values.iter().map(OsString::from).collect()
}

fn registered_state_worktree(repository_root: &Path) -> io::Result<PathBuf> {
    let output = run_git(repository_root, &args(&["worktree", "list", "--porcelain"]))?;
    let listing = String::from_utf8_lossy(&output.stdout);
    let matches: Vec<PathBuf> = listing
        .split("\n\n")
        .filter(|entry| {
            entry
                .lines()
                .any(|line| line == format!("branch {STATE_BRANCH_REF}"))
        })
        .filter_map(|entry| {
            entry
                .lines()
                .find_map(|line| line.strip_prefix("worktree ").map(PathBuf::from))
        })
        .collect();
    match matches.as_slice() {
        [path] => Ok(path.clone()),
        [] => Err(io::Error::new(
            io::ErrorKind::NotFound,
            "no registered waap worktree; run 'waap init'",
        )),
        _ => Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "multiple worktrees are registered for the waap branch",
        )),
    }
}

fn repair_worktree_link(repository_root: &Path, worktree: &Path) -> io::Result<()> {
    run_git(
        repository_root,
        &[
            "worktree".into(),
            "repair".into(),
            worktree.as_os_str().to_os_string(),
        ],
    )?;
    Ok(())
}

fn configure_upstream(repository_root: &Path) -> io::Result<()> {
    let remotes = run_git(repository_root, &args(&["remote"]))?;
    if String::from_utf8_lossy(&remotes.stdout)
        .lines()
        .any(|remote| remote == "origin")
    {
        run_git(
            repository_root,
            &args(&["config", "branch.waap.remote", "origin"]),
        )?;
        run_git(
            repository_root,
            &args(&["config", "branch.waap.merge", STATE_BRANCH_REF]),
        )?;
    }
    Ok(())
}

fn validate_state_worktree(repository_root: &Path, state_root: &Path) -> io::Result<()> {
    let registered = registered_state_worktree(repository_root)?;
    let registered = registered.canonicalize()?;
    let expected = state_root.canonicalize()?;
    if registered != expected {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!(
                "waap worktree is registered at {}, expected {}",
                registered.display(),
                expected.display()
            ),
        ));
    }
    let branch = run_git(state_root, &args(&["branch", "--show-current"]))?;
    if String::from_utf8_lossy(&branch.stdout).trim() != "waap" {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!(
                "{} does not check out the waap branch",
                state_root.display()
            ),
        ));
    }
    Ok(())
}

pub(crate) fn repair_project(
    repository_root: &Path,
    state_root: &Path,
    explicit_state_root: bool,
) -> io::Result<RepairReport> {
    if explicit_state_root {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "waap repair does not accept --waap-root",
        ));
    }

    let registered = registered_state_worktree(repository_root)?;
    let at_expected_path =
        state_root.exists() && registered.canonicalize()? == state_root.canonicalize()?;
    let relocated_from = if at_expected_path {
        repair_worktree_link(repository_root, state_root)?;
        None
    } else {
        if state_root.exists() {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                format!(
                    "cannot relocate waap state from {}: destination {} already exists",
                    registered.display(),
                    state_root.display()
                ),
            ));
        }
        if !registered.is_dir() {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                format!(
                    "registered waap worktree {} is missing",
                    registered.display()
                ),
            ));
        }
        repair_worktree_link(repository_root, &registered)?;
        if let Some(parent) = state_root.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::rename(&registered, state_root)?;
        if let Err(error) = repair_worktree_link(repository_root, state_root) {
            return match fs::rename(state_root, &registered) {
                Ok(()) => match repair_worktree_link(repository_root, &registered) {
                    Ok(()) => Err(error),
                    Err(repair_error) => Err(io::Error::new(
                        error.kind(),
                        format!("{error}; restored the state directory but failed to repair its registration: {repair_error}"),
                    )),
                },
                Err(rollback_error) => Err(io::Error::new(
                    error.kind(),
                    format!("{error}; failed to move state back: {rollback_error}"),
                )),
            };
        }
        Some(registered)
    };

    validate_state_worktree(repository_root, state_root)?;
    configure_upstream(repository_root)?;
    Ok(RepairReport {
        state_directory: state_root.canonicalize()?,
        relocated_from,
    })
}

pub(crate) fn print_repair_report(output_format: &OutputFormat, report: &RepairReport) {
    match output_format {
        OutputFormat::Json => println!(
            "{}",
            json!({
                "state_directory": report.state_directory.display().to_string(),
                "relocated_from": report
                    .relocated_from
                    .as_ref()
                    .map(|path| path.display().to_string()),
            })
        ),
        OutputFormat::HumanReadable => {
            println!("State directory: {}", report.state_directory.display());
            if let Some(source) = &report.relocated_from {
                println!("Relocated from: {}", source.display());
            } else {
                println!("Waap state is already repaired");
            }
        }
    }
}
