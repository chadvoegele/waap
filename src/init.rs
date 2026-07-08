use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde_json::json;

use crate::cli::OutputFormat;
use crate::git::{commit_paths, is_inside_git_work_tree};
use crate::mutation::{Committed, MutationError, MutationResult};

#[derive(Debug)]
pub(crate) struct InitReport {
    pub(crate) path: PathBuf,
    pub(crate) marker: PathBuf,
}

pub(crate) fn init_project(waap_root: &Path) -> MutationResult<Committed<InitReport>> {
    let waap_dir = waap_root.join(".waap");
    if waap_dir.exists() {
        return Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            format!("{} already exists", waap_dir.display()),
        )
        .into());
    }

    if !is_inside_git_work_tree(waap_root)? {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!(
                "{} is not inside a git repository; waap projects must be inside git",
                waap_root.display()
            ),
        )
        .into());
    }

    fs::create_dir_all(waap_dir.join("agents"))?;
    fs::create_dir_all(waap_dir.join("tickets"))?;
    let marker = waap_dir.join(".gitkeep");
    fs::write(&marker, "")?;

    let path = waap_root
        .canonicalize()
        .unwrap_or_else(|_| waap_root.to_path_buf());

    let report = InitReport { path, marker };
    let commit = commit_paths(waap_root, &[report.marker.as_path()], "waap init")
        .map_err(MutationError::Commit)?;

    Ok(Committed {
        value: report,
        commit,
    })
}

pub(crate) fn print_init_report(output_format: &OutputFormat, committed: &Committed<InitReport>) {
    let report = &committed.value;
    match output_format {
        OutputFormat::Json => {
            println!(
                "{}",
                json!({
                    "path": report.path.display().to_string(),
                    "commit": committed.commit,
                })
            );
        }
        OutputFormat::HumanReadable => {
            println!("Initialized waap project at {}", report.path.display());
            println!("Commit: {}", committed.commit);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use tempfile::tempdir;

    use super::init_project;
    use crate::check::check_waap;
    use crate::mutation::MutationError;
    use crate::test_git::init_repo;

    #[test]
    fn init_creates_waap_skeleton_in_fresh_git_repo() {
        let dir = tempdir().unwrap();
        init_repo(dir.path());

        let committed = init_project(dir.path()).unwrap();
        let report = committed.value;

        assert!(dir.path().join(".waap").is_dir());
        assert!(dir.path().join(".waap/agents").is_dir());
        assert!(dir.path().join(".waap/tickets").is_dir());
        assert_eq!(report.path, dir.path().canonicalize().unwrap());
        assert_eq!(report.marker, dir.path().join(".waap/.gitkeep"));
        assert!(!committed.commit.is_empty());
        assert!(check_waap(dir.path()).is_empty());
    }

    #[test]
    fn init_errors_when_waap_already_exists() {
        let dir = tempdir().unwrap();
        init_repo(dir.path());
        fs::create_dir_all(dir.path().join(".waap")).unwrap();

        let MutationError::Operation(err) = init_project(dir.path()).unwrap_err() else {
            panic!("expected initialization error");
        };

        assert_eq!(err.kind(), std::io::ErrorKind::AlreadyExists);
        assert!(err.to_string().contains(".waap"));
    }

    #[test]
    fn init_errors_outside_git_repository() {
        let dir = tempdir().unwrap();

        let MutationError::Operation(err) = init_project(dir.path()).unwrap_err() else {
            panic!("expected initialization error");
        };

        assert_eq!(err.kind(), std::io::ErrorKind::InvalidInput);
        assert!(!dir.path().join(".waap").exists());
    }
}
