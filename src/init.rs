use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde_json::json;

use crate::check::check_waap;
use crate::cli::OutputFormat;
use crate::git::{is_inside_git_work_tree, Committed, StateMutationContext, StateTransaction};

#[derive(Debug)]
pub(crate) struct InitReport {
    pub(crate) path: PathBuf,
    pub(crate) marker: PathBuf,
}

pub(crate) fn init_project(waap_root: &Path) -> io::Result<Committed<InitReport>> {
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

    let context = StateMutationContext::legacy(waap_root)?;
    let mut transaction = StateTransaction::begin(context, check_uninitialized_or_valid_waap)?;
    let waap_dir = transaction.state_root().join(".waap");
    let marker = waap_dir.join(".gitkeep");
    transaction.snapshot_path(&marker)?;
    fs::create_dir_all(waap_dir.join("agents"))?;
    fs::create_dir_all(waap_dir.join("tickets"))?;
    fs::write(&marker, "")?;

    let path = transaction.state_root().to_path_buf();
    let report = InitReport { path, marker };
    let commit = transaction
        .commit(&[report.marker.as_path()], "waap init")
        .map_err(|error| {
            io::Error::new(
                error.kind(),
                format!("failed to commit waap state change: {error}"),
            )
        })?;

    Ok(Committed {
        value: report,
        commit,
    })
}

fn check_uninitialized_or_valid_waap(root: &Path) -> Vec<String> {
    if root.join(".waap").exists() {
        check_waap(root)
    } else {
        Vec::new()
    }
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
