use std::env;
use std::ffi::OsStr;
use std::io;
use std::path::{Component, Path, PathBuf};
use std::process::Command;

#[derive(Debug)]
pub(crate) struct ControlRoot {
    runtime_dir: PathBuf,
    path: PathBuf,
}

impl ControlRoot {
    pub(crate) fn runtime_dir(&self) -> &Path {
        &self.runtime_dir
    }

    pub(crate) fn path(&self) -> &Path {
        &self.path
    }
}

pub(crate) fn find_control_root(repository_root: &Path) -> io::Result<ControlRoot> {
    find_control_root_in(repository_root, env::var_os("XDG_RUNTIME_DIR").as_deref())
}

pub(crate) fn find_control_root_in(
    repository_root: &Path,
    runtime_dir: Option<&OsStr>,
) -> io::Result<ControlRoot> {
    let runtime_dir = runtime_dir
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                "XDG_RUNTIME_DIR must be set to a private runtime directory",
            )
        })?;
    if !runtime_dir.is_absolute() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "XDG_RUNTIME_DIR must be an absolute path",
        ));
    }
    let path = runtime_dir
        .join("waap/control")
        .join(repository_relative_path(repository_root)?);
    Ok(ControlRoot { runtime_dir, path })
}

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

    let repository_path = repository_relative_path(start)?;
    let home = env::var_os("HOME")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .ok_or_else(|| {
            io::Error::new(io::ErrorKind::InvalidInput, "HOME must be an absolute path")
        })?;
    Ok(home.join(".local/state/waap/data").join(repository_path))
}

pub(crate) fn repository_relative_path(start: &Path) -> io::Result<PathBuf> {
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
    Ok(primary_repository_root
        .components()
        .filter_map(|component| match component {
            Component::Normal(component) => Some(component),
            _ => None,
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use std::env;
    use std::fs;
    use std::path::PathBuf;

    use tempfile::tempdir;

    use super::{find_control_root_in, find_git_root, find_waap_root};
    use crate::test_git::{init_repo_with_commit, run as git};

    #[test]
    fn control_root_uses_injected_runtime_and_primary_repository_identity() {
        let dir = tempdir().unwrap();
        let repo = dir.path().join("repo");
        let other = dir.path().join("other");
        let linked = dir.path().join("linked");
        for path in [&repo, &other] {
            fs::create_dir(path).unwrap();
            init_repo_with_commit(path);
        }
        git(
            &repo,
            &["worktree", "add", linked.to_str().unwrap(), "-b", "linked"],
        );
        let runtime = dir.path().join("runtime");
        let resolve =
            |path: &std::path::Path| find_control_root_in(path, Some(runtime.as_os_str())).unwrap();
        let control_root = resolve(&repo);
        assert_eq!(control_root.runtime_dir(), runtime);
        assert_eq!(
            control_root.path(),
            runtime
                .join("waap/control")
                .join(repo.canonicalize().unwrap().strip_prefix("/").unwrap())
        );
        assert_eq!(resolve(&linked).path(), control_root.path());
        assert_ne!(resolve(&other).path(), control_root.path());
        assert!(!runtime.exists(), "resolution must not create directories");
    }

    #[test]
    fn control_root_rejects_missing_empty_and_relative_runtime() {
        use std::ffi::OsStr;

        let repo = tempdir().unwrap();
        init_repo_with_commit(repo.path());
        for (runtime, message) in [
            (None, "XDG_RUNTIME_DIR must be set"),
            (Some(OsStr::new("")), "XDG_RUNTIME_DIR must be set"),
            (
                Some(OsStr::new("relative")),
                "XDG_RUNTIME_DIR must be an absolute path",
            ),
        ] {
            let error = find_control_root_in(repo.path(), runtime).unwrap_err();
            assert_eq!(error.kind(), std::io::ErrorKind::InvalidInput);
            assert!(error.to_string().contains(message));
        }
    }

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
