use std::env;
use std::io;
use std::path::{Component, Path, PathBuf};
use std::process::Command;

fn not_in_repository() -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, "not inside a git repository")
}

/// Find the application worktree containing `start`.
pub(crate) fn find_git_root(start: &Path) -> io::Result<PathBuf> {
    let mut current = start.canonicalize()?;
    loop {
        if current.join(".git").exists() {
            return Ok(current);
        }
        current = current
            .parent()
            .ok_or_else(not_in_repository)?
            .to_path_buf();
    }
}

/// Find the dedicated waap state worktree for the repository containing `start`.
pub(crate) fn find_waap_root(
    start: &Path,
    explicit_waap_root: Option<&Path>,
) -> io::Result<PathBuf> {
    if let Some(explicit) = explicit_waap_root {
        let path = if explicit.is_absolute() {
            explicit.to_path_buf()
        } else {
            start.join(explicit)
        };
        return if path.exists() {
            path.canonicalize()
        } else {
            Ok(path)
        };
    }

    let repository_root = find_git_root(start)?;
    let mut command = Command::new("git");
    command.current_dir(&repository_root).args([
        "rev-parse",
        "--path-format=absolute",
        "--git-common-dir",
    ]);
    #[cfg(test)]
    crate::test_git::isolate(&mut command);
    let output = command.output()?;
    if !output.status.success() {
        return Err(not_in_repository());
    }
    let common_git_dir =
        PathBuf::from(String::from_utf8_lossy(&output.stdout).trim().to_string()).canonicalize()?;
    if common_git_dir.file_name().and_then(|name| name.to_str()) != Some(".git") {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!(
                "unsupported git directory {}; expected <repository>/.git",
                common_git_dir.display()
            ),
        ));
    }
    let primary_repository_root = common_git_dir.parent().ok_or_else(not_in_repository)?;
    let home = env::var_os("HOME")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .ok_or_else(|| {
            io::Error::new(io::ErrorKind::InvalidInput, "HOME must be an absolute path")
        })?;
    let mut waap_root = home.join(".local/state/waap/data");
    for component in primary_repository_root.components() {
        if let Component::Normal(component) = component {
            waap_root.push(component);
        }
    }
    Ok(waap_root)
}

#[cfg(test)]
mod tests {
    use std::env;
    use std::fs;
    use std::path::PathBuf;

    use tempfile::tempdir;

    use super::{find_git_root, find_waap_root};
    use crate::test_git::{init_repo_with_commit, run as git};

    #[test]
    fn derives_state_path_from_primary_repository() {
        let dir = tempdir().unwrap();
        let repo = dir.path().join("repo");
        fs::create_dir_all(&repo).unwrap();
        init_repo_with_commit(&repo);

        assert_eq!(find_git_root(&repo).unwrap(), repo.canonicalize().unwrap());
        assert_eq!(
            find_waap_root(&repo, None).unwrap(),
            PathBuf::from(env::var_os("HOME").unwrap())
                .join(".local/state/waap/data")
                .join(repo.canonicalize().unwrap().strip_prefix("/").unwrap())
        );
    }

    #[test]
    fn linked_worktree_uses_primary_repository_state() {
        let dir = tempdir().unwrap();
        let repo = dir.path().join("repo");
        let linked = dir.path().join("linked");
        fs::create_dir_all(&repo).unwrap();
        init_repo_with_commit(&repo);
        git(
            &repo,
            &["worktree", "add", linked.to_str().unwrap(), "-b", "feature"],
        );

        assert_eq!(
            find_git_root(&linked).unwrap(),
            linked.canonicalize().unwrap()
        );
        assert_eq!(
            find_waap_root(&linked, None).unwrap(),
            find_waap_root(&repo, None).unwrap()
        );
    }

    #[test]
    fn explicit_root_is_used_as_state_directory() {
        let dir = tempdir().unwrap();
        init_repo_with_commit(dir.path());
        let state = dir.path().join("state");

        assert_eq!(find_waap_root(dir.path(), Some(&state)).unwrap(), state);
    }
}
