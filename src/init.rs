use std::io;
use std::path::PathBuf;

use serde_json::json;

use crate::cli::OutputFormat;
use crate::git::{initialize_state_worktree, state_worktree_head, Committed};
use crate::root::ProjectContext;

#[derive(Debug)]
pub(crate) struct InitReport {
    pub(crate) state_directory: PathBuf,
}

/// Set up the dedicated state worktree without touching the application
/// checkout. Legacy state is deliberately left for `waap repair`.
pub(crate) fn init_project(
    context: &ProjectContext,
    has_explicit_state_root: bool,
) -> io::Result<Committed<InitReport>> {
    let legacy_state = context.invocation_worktree_root.join(".waap");
    let selected_state_exists = context.state_root.exists();

    if !has_explicit_state_root && legacy_state.exists() {
        let detail = if selected_state_exists {
            format!(
                "state directory {} and legacy state {} already exist; use waap repair",
                context.state_root.display(),
                legacy_state.display()
            )
        } else {
            format!(
                "legacy state {} already exists; waap init is setup-only, use waap repair",
                legacy_state.display()
            )
        };
        return Err(io::Error::new(io::ErrorKind::AlreadyExists, detail));
    }

    if selected_state_exists {
        return Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            format!(
                "state directory {} already exists; waap init is setup-only, use waap repair",
                context.state_root.display()
            ),
        ));
    }

    let state_directory =
        initialize_state_worktree(&context.primary_repository_root, &context.state_root)?;
    let commit = state_worktree_head(&state_directory)?;

    Ok(Committed {
        value: InitReport { state_directory },
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
                    "state_directory": report.state_directory.display().to_string(),
                    "commit": committed.commit,
                })
            );
        }
        OutputFormat::HumanReadable => {
            println!("State directory: {}", report.state_directory.display());
            println!("Initialized waap project");
            println!("Commit: {}", committed.commit);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use tempfile::tempdir;

    use super::init_project;
    use crate::root::resolve_init_project_context;
    use crate::test_git::{init_repo, init_repo_with_commit, run};

    #[test]
    fn init_creates_central_skeleton_without_changing_application_head() {
        let repository = tempdir().unwrap();
        let state_parent = tempdir().unwrap();
        let state = state_parent.path().join("state");
        init_repo_with_commit(repository.path());
        let application_head = run(repository.path(), &["rev-parse", "HEAD"]);
        let context = resolve_init_project_context(repository.path(), Some(&state)).unwrap();

        let committed = init_project(&context, true).unwrap();

        assert_eq!(
            committed.value.state_directory,
            state.canonicalize().unwrap()
        );
        assert!(state.join("agents").is_dir());
        assert!(state.join("tickets").is_dir());
        assert_eq!(
            run(repository.path(), &["rev-parse", "HEAD"]),
            application_head
        );
        assert!(!committed.commit.is_empty());
    }

    #[test]
    fn init_rejects_legacy_state_without_modifying_the_selected_state() {
        let repository = tempdir().unwrap();
        init_repo(repository.path());
        fs::create_dir_all(repository.path().join(".waap/agents")).unwrap();
        let context = resolve_init_project_context(repository.path(), None).unwrap();

        let error = init_project(&context, false).unwrap_err();

        assert!(error.to_string().contains("legacy state"));
        assert!(!context.state_root.exists());
    }
}
