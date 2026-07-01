use std::ffi::OsString;
use std::io;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

/// Relative path, under the repository root, of the worktree that `waap agent run`
/// prepares for an agent.
pub(crate) fn agent_worktree_dir(agent_id: &str) -> PathBuf {
    Path::new("worktrees").join(agent_id)
}

/// Create the git worktree that `waap agent run` runs the agent inside.
///
/// A fresh branch named after the agent is created at the current `HEAD` and checked out into
/// `worktrees/<agent_id>`. Returns the canonical absolute path of the new worktree so callers can
/// launch the selected system there.
pub(crate) fn create_agent_worktree(waap_root: &Path, agent_id: &str) -> io::Result<PathBuf> {
    let relative = agent_worktree_dir(agent_id);
    run_git(
        waap_root,
        &[
            "worktree".into(),
            "add".into(),
            "-b".into(),
            agent_id.into(),
            relative.as_os_str().to_os_string(),
        ],
    )?;
    waap_root.join(&relative).canonicalize()
}

/// Remove the agent worktree created by [`create_agent_worktree`].
///
/// `--force` is used so cleanup still succeeds when the agent left uncommitted or untracked changes
/// behind, which keeps the worktree lifecycle consistent even after an early exit or failure.
pub(crate) fn remove_agent_worktree(waap_root: &Path, agent_id: &str) -> io::Result<()> {
    let relative = agent_worktree_dir(agent_id);
    run_git(
        waap_root,
        &[
            "worktree".into(),
            "remove".into(),
            "--force".into(),
            relative.as_os_str().to_os_string(),
        ],
    )?;
    Ok(())
}

/// Stage and commit only the given paths under `waap_root`, returning the new commit hash.
///
/// A pathspec is passed to both `git add` and `git commit` so that unrelated changes already
/// present in the working tree or index are left untouched: the commit records the working-tree
/// contents of `paths` and nothing else. All git invocations run with `waap_root` as their working
/// directory so `--waap-root` is respected.
pub(crate) fn commit_paths(waap_root: &Path, paths: &[&Path], message: &str) -> io::Result<String> {
    if paths.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "no paths to commit",
        ));
    }

    let mut add_args: Vec<OsString> = vec!["add".into(), "--".into()];
    add_args.extend(paths.iter().map(|path| path.as_os_str().to_os_string()));
    run_git(waap_root, &add_args)?;

    // A no-op state write (the paths have no staged diff) is a successful no-op: skip the commit so
    // idempotent state writes don't abort on git's "nothing to commit" non-zero exit. `git diff
    // --cached --quiet` signals via its exit code (0 = nothing staged, 1 = staged changes), so it
    // must bypass the success-only `run_git` wrapper.
    let mut diff_args: Vec<OsString> = vec![
        "diff".into(),
        "--cached".into(),
        "--quiet".into(),
        "--".into(),
    ];
    diff_args.extend(paths.iter().map(|path| path.as_os_str().to_os_string()));
    let diff = git_command(waap_root, &diff_args)?;
    let has_staged_changes = match diff.status.code() {
        Some(0) => false,
        Some(1) => true,
        // Any other exit status (or a signal) is a genuine failure, not the documented diff signal.
        _ => return Err(run_git_error(&diff_args, &diff)),
    };

    if has_staged_changes {
        let mut commit_args: Vec<OsString> =
            vec!["commit".into(), "-m".into(), message.into(), "--".into()];
        commit_args.extend(paths.iter().map(|path| path.as_os_str().to_os_string()));
        run_git(waap_root, &commit_args)?;
    }

    let output = run_git(waap_root, &["rev-parse".into(), "HEAD".into()])?;
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

/// Whether `path` is inside a git working tree.
pub(crate) fn is_inside_git_work_tree(path: &Path) -> io::Result<bool> {
    let output = Command::new("git")
        .current_dir(path)
        .args(["rev-parse", "--is-inside-work-tree"])
        .output()
        .map_err(|error| io::Error::new(error.kind(), format!("failed to run git: {error}")))?;
    Ok(output.status.success() && String::from_utf8_lossy(&output.stdout).trim() == "true")
}

/// Run `git` in `waap_root` and return its raw [`Output`] without treating a non-zero exit as an
/// error, so callers can inspect the exit code themselves.
fn git_command(waap_root: &Path, args: &[OsString]) -> io::Result<Output> {
    Command::new("git")
        .current_dir(waap_root)
        .args(args)
        .output()
        .map_err(|error| io::Error::new(error.kind(), format!("failed to run git: {error}")))
}

/// Build the error for a `git` invocation that exited unsuccessfully.
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

fn run_git(waap_root: &Path, args: &[OsString]) -> io::Result<Output> {
    let output = git_command(waap_root, args)?;

    if !output.status.success() {
        return Err(run_git_error(args, &output));
    }

    Ok(output)
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::Path;
    use std::process::Command;

    use tempfile::tempdir;

    use super::{
        commit_paths, create_agent_worktree, is_inside_git_work_tree, remove_agent_worktree,
    };

    fn init_repo(root: &Path) {
        run(root, &["init", "-q"]);
        run(root, &["config", "user.name", "Test"]);
        run(root, &["config", "user.email", "test@example.com"]);
    }

    fn init_repo_with_commit(root: &Path) {
        init_repo(root);
        write_file(&root.join("README.md"), "seed\n");
        run(root, &["add", "-A"]);
        run(root, &["commit", "-q", "-m", "seed"]);
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

    fn write_file(path: &Path, contents: &str) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, contents).unwrap();
    }

    #[test]
    fn commit_paths_creates_single_commit_with_returned_hash() {
        let dir = tempdir().unwrap();
        init_repo(dir.path());
        let file = dir.path().join(".waap/tickets/tt-x/ticket.md");
        write_file(&file, "+++\n+++\n");

        let count_before = run(dir.path(), &["rev-list", "--count", "--all"])
            .parse::<u32>()
            .unwrap_or(0);
        let hash = commit_paths(dir.path(), &[file.as_path()], "waap ticket new tt-x").unwrap();
        let count_after: u32 = run(dir.path(), &["rev-list", "--count", "HEAD"])
            .parse()
            .unwrap();

        assert_eq!(count_after, count_before + 1);
        assert_eq!(run(dir.path(), &["rev-parse", "HEAD"]), hash);
        assert_eq!(
            run(dir.path(), &["log", "-1", "--pretty=%s"]),
            "waap ticket new tt-x"
        );
    }

    #[test]
    fn commit_paths_only_stages_given_files() {
        let dir = tempdir().unwrap();
        init_repo(dir.path());
        // An unrelated file that is already staged must not be swept into the commit.
        let unrelated = dir.path().join("unrelated.txt");
        write_file(&unrelated, "user change\n");
        run(dir.path(), &["add", "unrelated.txt"]);

        let tracked = dir.path().join(".waap/agents/aa-00000001/agent.md");
        write_file(&tracked, "+++\n+++\n");

        commit_paths(
            dir.path(),
            &[tracked.as_path()],
            "waap agent new aa-00000001",
        )
        .unwrap();

        let committed = run(
            dir.path(),
            &["show", "--name-only", "--pretty=format:", "HEAD"],
        );
        assert!(committed.contains(".waap/agents/aa-00000001/agent.md"));
        assert!(!committed.contains("unrelated.txt"));
        // The unrelated change is still staged and uncommitted.
        let staged = run(dir.path(), &["diff", "--cached", "--name-only"]);
        assert!(staged.contains("unrelated.txt"));
    }

    #[test]
    fn commit_paths_commits_only_changed_path_among_many_modifications() {
        let dir = tempdir().unwrap();
        init_repo(dir.path());
        let a = dir.path().join(".waap/agents/aa-00000001/agent.md");
        let b = dir.path().join(".waap/agents/aa-00000002/agent.md");
        write_file(&a, "+++\nstatus = \"ready\"\n+++\n");
        write_file(&b, "+++\nstatus = \"ready\"\n+++\n");
        run(dir.path(), &["add", "-A"]);
        run(dir.path(), &["commit", "-q", "-m", "seed"]);

        // Modify both, but only commit one.
        write_file(&a, "+++\nstatus = \"running\"\n+++\n");
        write_file(&b, "+++\nstatus = \"running\"\n+++\n");

        commit_paths(dir.path(), &[a.as_path()], "waap agent run aa-00000001").unwrap();

        let committed = run(
            dir.path(),
            &["show", "--name-only", "--pretty=format:", "HEAD"],
        );
        assert!(committed.contains("aa-00000001"));
        assert!(!committed.contains("aa-00000002"));
    }

    #[test]
    fn commit_paths_noop_returns_head_without_new_commit() {
        let dir = tempdir().unwrap();
        init_repo(dir.path());
        let file = dir.path().join(".waap/agents/aa-00000001/agent.md");
        write_file(&file, "+++\nstatus = \"completed\"\n+++\n");
        // First write creates the commit; the second writes identical contents (no staged diff).
        let first =
            commit_paths(dir.path(), &[file.as_path()], "waap agent run aa-00000001").unwrap();

        let count_before = run(dir.path(), &["rev-list", "--count", "HEAD"])
            .parse::<u32>()
            .unwrap();
        let second =
            commit_paths(dir.path(), &[file.as_path()], "waap agent run aa-00000001").unwrap();
        let count_after = run(dir.path(), &["rev-list", "--count", "HEAD"])
            .parse::<u32>()
            .unwrap();

        assert_eq!(count_after, count_before, "no new commit should be created");
        assert_eq!(second, first, "the current HEAD is returned for a no-op");
        assert_eq!(run(dir.path(), &["rev-parse", "HEAD"]), second);
    }

    #[test]
    fn commit_paths_reports_failure_when_not_a_git_repo() {
        let dir = tempdir().unwrap();
        let file = dir.path().join(".waap/tickets/tt-x/ticket.md");
        write_file(&file, "+++\n+++\n");

        let error =
            commit_paths(dir.path(), &[file.as_path()], "waap ticket new tt-x").unwrap_err();

        assert!(!error.to_string().is_empty());
    }

    #[test]
    fn commit_paths_respects_waap_root() {
        let dir = tempdir().unwrap();
        let waap_root = dir.path().join("nested/repo");
        fs::create_dir_all(&waap_root).unwrap();
        init_repo(&waap_root);
        let file = waap_root.join(".waap/tickets/tt-x/ticket.md");
        write_file(&file, "+++\n+++\n");

        let hash = commit_paths(&waap_root, &[file.as_path()], "waap ticket new tt-x").unwrap();

        assert_eq!(run(&waap_root, &["rev-parse", "HEAD"]), hash);
        assert_eq!(
            run(&waap_root, &["log", "-1", "--pretty=%s"]),
            "waap ticket new tt-x"
        );
    }

    #[test]
    fn commit_paths_rejects_empty_paths() {
        let dir = tempdir().unwrap();
        init_repo(dir.path());

        let error = commit_paths(dir.path(), &[], "waap ticket new tt-x").unwrap_err();

        assert_eq!(error.kind(), std::io::ErrorKind::InvalidInput);
    }

    #[test]
    fn is_inside_git_work_tree_true_for_git_repo() {
        let dir = tempdir().unwrap();
        init_repo(dir.path());

        assert!(is_inside_git_work_tree(dir.path()).unwrap());
    }

    #[test]
    fn is_inside_git_work_tree_false_outside_git_repo() {
        let dir = tempdir().unwrap();

        assert!(!is_inside_git_work_tree(dir.path()).unwrap());
    }

    #[test]
    fn create_agent_worktree_creates_checkout_and_branch() {
        let dir = tempdir().unwrap();
        init_repo_with_commit(dir.path());

        let worktree = create_agent_worktree(dir.path(), "aa-00000001").unwrap();

        assert!(worktree.is_dir());
        assert_eq!(
            worktree,
            dir.path()
                .join("worktrees/aa-00000001")
                .canonicalize()
                .unwrap()
        );
        // The seed commit's tree is checked out in the worktree.
        assert!(worktree.join("README.md").exists());
        // A branch named after the agent now exists and is checked out in the worktree.
        let branches = run(dir.path(), &["branch", "--list", "aa-00000001"]);
        assert!(branches.contains("aa-00000001"));
        let worktrees = run(dir.path(), &["worktree", "list"]);
        assert!(worktrees.contains("worktrees/aa-00000001"));
    }

    #[test]
    fn remove_agent_worktree_deletes_checkout() {
        let dir = tempdir().unwrap();
        init_repo_with_commit(dir.path());
        let worktree = create_agent_worktree(dir.path(), "aa-00000001").unwrap();

        remove_agent_worktree(dir.path(), "aa-00000001").unwrap();

        assert!(!worktree.exists());
        let worktrees = run(dir.path(), &["worktree", "list"]);
        assert!(!worktrees.contains("worktrees/aa-00000001"));
    }

    #[test]
    fn remove_agent_worktree_forces_removal_with_uncommitted_changes() {
        let dir = tempdir().unwrap();
        init_repo_with_commit(dir.path());
        let worktree = create_agent_worktree(dir.path(), "aa-00000001").unwrap();
        // Leave dirty state behind, as an agent that exits early or fails would.
        write_file(&worktree.join("scratch.txt"), "uncommitted work\n");

        remove_agent_worktree(dir.path(), "aa-00000001").unwrap();

        assert!(!worktree.exists());
    }
}
