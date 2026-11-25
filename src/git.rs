use std::ffi::OsString;
use std::io;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

pub(crate) fn create_worktree(
    repo_root: &Path,
    branch: &str,
    relative_path: &Path,
) -> io::Result<PathBuf> {
    run_git(
        repo_root,
        &[
            "worktree".into(),
            "add".into(),
            "-b".into(),
            branch.into(),
            relative_path.as_os_str().to_os_string(),
        ],
    )?;
    repo_root.join(relative_path).canonicalize()
}

pub(crate) fn remove_worktree(repo_root: &Path, relative_path: &Path) -> io::Result<()> {
    run_git(
        repo_root,
        &[
            "worktree".into(),
            "remove".into(),
            "--force".into(), // remove worktrees with uncommitted or untracked changes
            relative_path.as_os_str().to_os_string(),
        ],
    )?;
    Ok(())
}

fn git_process(repository_root: &Path) -> Command {
    let mut command = Command::new("git");
    command.current_dir(repository_root);
    #[cfg(test)]
    crate::test_git::isolate(&mut command);
    command
}

fn git_command(repository_root: &Path, args: &[OsString]) -> io::Result<Output> {
    git_process(repository_root)
        .args(args)
        .output()
        .map_err(|error| io::Error::new(error.kind(), format!("failed to run git: {error}")))
}

fn run_git_error(args: &[OsString], output: &Output) -> io::Error {
    let stderr = String::from_utf8_lossy(&output.stderr);
    let stderr = stderr.trim();
    let subcommand = args
        .first()
        .map(|arg| arg.to_string_lossy().into_owned())
        .unwrap_or_default();
    let detail = if stderr.is_empty() {
        format!("git {subcommand} exited with {}", output.status)
    } else {
        format!("git {subcommand} failed: {stderr}")
    };
    io::Error::other(detail)
}

fn run_git(repository_root: &Path, args: &[OsString]) -> io::Result<Output> {
    let output = git_command(repository_root, args)?;

    if !output.status.success() {
        return Err(run_git_error(args, &output));
    }

    Ok(output)
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::Path;

    use tempfile::tempdir;

    use super::{create_worktree, remove_worktree};
    use crate::test_git::{init_repo_with_commit, run};

    fn write_file(path: &Path, contents: &str) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, contents).unwrap();
    }

    #[test]
    fn create_worktree_creates_checkout_at_requested_path_and_branch() {
        let dir = tempdir().unwrap();
        init_repo_with_commit(dir.path());
        let relative_path = Path::new("checkouts/topic");

        let worktree = create_worktree(dir.path(), "topic-branch", relative_path).unwrap();

        assert!(worktree.is_dir());
        assert_eq!(
            worktree,
            dir.path().join(relative_path).canonicalize().unwrap()
        );
        // The seed commit's tree is checked out in the worktree.
        assert!(worktree.join("README.md").exists());
        let branches = run(dir.path(), &["branch", "--list", "topic-branch"]);
        assert!(branches.contains("topic-branch"));
        let worktrees = run(dir.path(), &["worktree", "list"]);
        assert!(worktrees.contains("checkouts/topic"));
    }

    #[test]
    fn remove_worktree_deletes_requested_checkout() {
        let dir = tempdir().unwrap();
        init_repo_with_commit(dir.path());
        let relative_path = Path::new("checkouts/topic");
        let worktree = create_worktree(dir.path(), "topic-branch", relative_path).unwrap();

        remove_worktree(dir.path(), relative_path).unwrap();

        assert!(!worktree.exists());
        let worktrees = run(dir.path(), &["worktree", "list"]);
        assert!(!worktrees.contains("checkouts/topic"));
    }

    #[test]
    fn remove_worktree_forces_removal_with_uncommitted_changes() {
        let dir = tempdir().unwrap();
        init_repo_with_commit(dir.path());
        let relative_path = Path::new("checkouts/topic");
        let worktree = create_worktree(dir.path(), "topic-branch", relative_path).unwrap();
        // Leave dirty state behind, as an agent that exits early or fails would.
        write_file(&worktree.join("scratch.txt"), "uncommitted work\n");

        remove_worktree(dir.path(), relative_path).unwrap();

        assert!(!worktree.exists());
    }
}
