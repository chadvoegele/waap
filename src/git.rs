use std::ffi::OsString;
use std::io;
use std::path::Path;
use std::process::{Command, Output};

/// Stage and commit only the given paths under `repo_root`, returning the new commit hash.
///
/// A pathspec is passed to both `git add` and `git commit` so that unrelated changes already
/// present in the working tree or index are left untouched: the commit records the working-tree
/// contents of `paths` and nothing else. All git invocations run with `repo_root` as their working
/// directory so `--repo-root` is respected.
pub(crate) fn commit_paths(repo_root: &Path, paths: &[&Path], message: &str) -> io::Result<String> {
    if paths.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "no paths to commit",
        ));
    }

    let mut add_args: Vec<OsString> = vec!["add".into(), "--".into()];
    add_args.extend(paths.iter().map(|path| path.as_os_str().to_os_string()));
    run_git(repo_root, &add_args)?;

    let mut commit_args: Vec<OsString> =
        vec!["commit".into(), "-m".into(), message.into(), "--".into()];
    commit_args.extend(paths.iter().map(|path| path.as_os_str().to_os_string()));
    run_git(repo_root, &commit_args)?;

    let output = run_git(repo_root, &["rev-parse".into(), "HEAD".into()])?;
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

fn run_git(repo_root: &Path, args: &[OsString]) -> io::Result<Output> {
    let output = Command::new("git")
        .current_dir(repo_root)
        .args(args)
        .output()
        .map_err(|error| io::Error::new(error.kind(), format!("failed to run git: {error}")))?;

    if !output.status.success() {
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
        return Err(io::Error::other(detail));
    }

    Ok(output)
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::Path;
    use std::process::Command;

    use tempfile::tempdir;

    use super::commit_paths;

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
    fn commit_paths_reports_failure_when_not_a_git_repo() {
        let dir = tempdir().unwrap();
        let file = dir.path().join(".waap/tickets/tt-x/ticket.md");
        write_file(&file, "+++\n+++\n");

        let error =
            commit_paths(dir.path(), &[file.as_path()], "waap ticket new tt-x").unwrap_err();

        assert!(!error.to_string().is_empty());
    }

    #[test]
    fn commit_paths_respects_repo_root() {
        let dir = tempdir().unwrap();
        let repo_root = dir.path().join("nested/repo");
        fs::create_dir_all(&repo_root).unwrap();
        init_repo(&repo_root);
        let file = repo_root.join(".waap/tickets/tt-x/ticket.md");
        write_file(&file, "+++\n+++\n");

        let hash = commit_paths(&repo_root, &[file.as_path()], "waap ticket new tt-x").unwrap();

        assert_eq!(run(&repo_root, &["rev-parse", "HEAD"]), hash);
        assert_eq!(
            run(&repo_root, &["log", "-1", "--pretty=%s"]),
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
}
