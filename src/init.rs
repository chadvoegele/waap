use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde_json::json;

use crate::cli::OutputFormat;
use crate::git::is_inside_git_work_tree;

#[derive(Debug)]
pub(crate) struct InitReport {
    pub(crate) path: PathBuf,
    pub(crate) marker: PathBuf,
}

/// Create the `.waap/` project skeleton at `waap_root`.
///
/// `agents/` and `tickets/` start out empty, so a marker file is written directly under `.waap/`
/// to give the initial commit something to track — `check_waap` only inspects the `agents` and
/// `tickets` subdirectories, so the marker doesn't trip its validation.
pub(crate) fn init_project(waap_root: &Path) -> io::Result<InitReport> {
    let waap_dir = waap_root.join(".waap");
    if waap_dir.exists() {
        return Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            format!("{} already exists", waap_dir.display()),
        ));
    }

    if !is_inside_git_work_tree(waap_root)? {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!(
                "{} is not inside a git repository; waap projects must be inside git",
                waap_root.display()
            ),
        ));
    }

    fs::create_dir_all(waap_dir.join("agents"))?;
    fs::create_dir_all(waap_dir.join("tickets"))?;
    let marker = waap_dir.join(".gitkeep");
    fs::write(&marker, "")?;

    let path = waap_root
        .canonicalize()
        .unwrap_or_else(|_| waap_root.to_path_buf());

    Ok(InitReport { path, marker })
}

pub(crate) fn print_init_report(output_format: &OutputFormat, report: &InitReport, commit: &str) {
    match output_format {
        OutputFormat::Json => {
            println!(
                "{}",
                json!({
                    "path": report.path.display().to_string(),
                    "commit": commit,
                })
            );
        }
        OutputFormat::HumanReadable => {
            println!("Initialized waap project at {}", report.path.display());
            println!("Commit: {commit}");
        }
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::Path;
    use std::process::Command;

    use tempfile::tempdir;

    use super::init_project;
    use crate::check::check_waap;

    fn init_repo(root: &Path) {
        run(root, &["init", "-q"]);
        run(root, &["config", "user.name", "Test"]);
        run(root, &["config", "user.email", "test@example.com"]);
    }

    fn run(root: &Path, args: &[&str]) -> String {
        let output = Command::new("git")
            .current_dir(root)
            .args(args)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "git {args:?} failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8_lossy(&output.stdout).trim().to_string()
    }

    #[test]
    fn init_creates_waap_skeleton_in_fresh_git_repo() {
        let dir = tempdir().unwrap();
        init_repo(dir.path());

        let report = init_project(dir.path()).unwrap();

        assert!(dir.path().join(".waap").is_dir());
        assert!(dir.path().join(".waap/agents").is_dir());
        assert!(dir.path().join(".waap/tickets").is_dir());
        assert_eq!(report.path, dir.path().canonicalize().unwrap());
        assert_eq!(report.marker, dir.path().join(".waap/.gitkeep"));
        assert!(check_waap(dir.path()).is_empty());
    }

    #[test]
    fn init_errors_when_waap_already_exists() {
        let dir = tempdir().unwrap();
        init_repo(dir.path());
        fs::create_dir_all(dir.path().join(".waap")).unwrap();

        let err = init_project(dir.path()).unwrap_err();

        assert_eq!(err.kind(), std::io::ErrorKind::AlreadyExists);
        assert!(err.to_string().contains(".waap"));
    }

    #[test]
    fn init_errors_outside_git_repository() {
        let dir = tempdir().unwrap();

        let err = init_project(dir.path()).unwrap_err();

        assert_eq!(err.kind(), std::io::ErrorKind::InvalidInput);
        assert!(!dir.path().join(".waap").exists());
    }
}
