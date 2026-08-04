use std::env;
use std::fs;
use std::io;
use std::path::{Component, Path, PathBuf};

/// Repository and state paths resolved for one waap invocation.
///
/// State operations use `state_root`; source operations use
/// `invocation_worktree_root`. Keeping them distinct prevents an agent source
/// worktree from accidentally being created from the state worktree's branch.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ProjectContext {
    pub(crate) invocation_worktree_root: PathBuf,
    pub(crate) primary_repository_root: PathBuf,
    pub(crate) common_git_dir: PathBuf,
    pub(crate) state_root: PathBuf,
}

impl ProjectContext {
    /// Return the application checkout from which source worktrees may be made.
    ///
    /// The state checkout has no selected application source HEAD, so it cannot
    /// be used for agent source operations.
    pub(crate) fn application_source_root(&self) -> io::Result<&Path> {
        if self.invocation_worktree_root == self.state_root {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!(
                    "state worktree {} cannot be used as an application source; invoke waap from an application worktree",
                    self.state_root.display()
                ),
            ));
        }
        Ok(&self.invocation_worktree_root)
    }
}

/// Whether an explicitly selected state directory must already be usable.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum StateRootRequirement {
    Existing,
    MayBeMissing,
}

/// Resolve repository and state paths for one command invocation.
pub(crate) fn resolve_project_context(
    start: &Path,
    explicit_state_root: Option<&Path>,
    requirement: StateRootRequirement,
) -> io::Result<ProjectContext> {
    let canonical_start = start.canonicalize()?;
    let invocation_worktree_root = find_invocation_worktree_root(&canonical_start)?;
    let common_git_dir = resolve_common_git_dir(&invocation_worktree_root)?;
    let primary_repository_root = resolve_primary_repository_root(&common_git_dir)?;
    let state_root = match explicit_state_root {
        Some(path) => resolve_explicit_state_root(&canonical_start, path, requirement)?,
        None => state_root_for_primary(&primary_repository_root, &home_directory()?)?,
    };

    Ok(ProjectContext {
        invocation_worktree_root,
        primary_repository_root,
        common_git_dir,
        state_root,
    })
}

fn find_invocation_worktree_root(start: &Path) -> io::Result<PathBuf> {
    let mut current = start;
    loop {
        if current.join(".git").exists() {
            return Ok(current.to_path_buf());
        }
        match current.parent() {
            Some(parent) => current = parent,
            None => {
                if find_bare_git_directory(start).is_some() {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidInput,
                        "unsupported bare repository; waap requires a primary checkout with a .git directory",
                    ));
                }
                return Err(not_inside_git_repository_error());
            }
        }
    }
}

fn find_bare_git_directory(start: &Path) -> Option<&Path> {
    let mut current = start;
    loop {
        if current.join("HEAD").is_file()
            && current.join("objects").is_dir()
            && current.join("refs").is_dir()
        {
            return Some(current);
        }
        current = current.parent()?;
    }
}

fn resolve_common_git_dir(invocation_worktree_root: &Path) -> io::Result<PathBuf> {
    let git_entry = invocation_worktree_root.join(".git");
    if git_entry.is_dir() {
        return git_entry.canonicalize();
    }
    if !git_entry.is_file() {
        return Err(not_inside_git_repository_error());
    }

    let git_dir = resolve_gitdir_file(&git_entry)?;
    let commondir_file = git_dir.join("commondir");
    if !commondir_file.is_file() {
        return Err(unsupported_separate_git_dir_error(&git_dir));
    }
    let commondir = read_commondir_file(&commondir_file)?;
    let common_git_dir = if commondir.is_absolute() {
        commondir
    } else {
        git_dir.join(commondir)
    };
    common_git_dir.canonicalize().map_err(|error| {
        io::Error::new(
            error.kind(),
            format!(
                "failed to resolve common Git directory from {}: {error}",
                commondir_file.display()
            ),
        )
    })
}

fn read_commondir_file(path: &Path) -> io::Result<PathBuf> {
    let contents = fs::read_to_string(path)?;
    let value = contents.trim();
    if value.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!(
                "invalid {}: expected a common Git directory path",
                path.display()
            ),
        ));
    }
    Ok(PathBuf::from(value))
}

fn resolve_gitdir_file(git_file: &Path) -> io::Result<PathBuf> {
    let git_dir = read_git_path_file(git_file, "gitdir:")?;
    let git_dir = if git_dir.is_absolute() {
        git_dir
    } else {
        git_file
            .parent()
            .unwrap_or_else(|| Path::new(""))
            .join(git_dir)
    };
    git_dir.canonicalize().map_err(|error| {
        io::Error::new(
            error.kind(),
            format!(
                "failed to resolve gitdir from {}: {error}; if this linked worktree followed a repository move, run waap repair from the primary repository",
                git_file.display()
            ),
        )
    })
}

fn read_git_path_file(path: &Path, prefix: &str) -> io::Result<PathBuf> {
    let contents = fs::read_to_string(path)?;
    let value = contents
        .lines()
        .next()
        .and_then(|line| line.strip_prefix(prefix))
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                format!("invalid {}: expected {prefix} <path>", path.display()),
            )
        })?;
    Ok(PathBuf::from(value))
}

fn resolve_primary_repository_root(common_git_dir: &Path) -> io::Result<PathBuf> {
    if common_git_dir.file_name().is_none_or(|name| name != ".git") {
        return Err(unsupported_separate_git_dir_error(common_git_dir));
    }
    let primary_repository_root = common_git_dir.parent().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            format!(
                "unsupported separate-Git-dir repository: common Git directory {} has no primary checkout",
                common_git_dir.display()
            ),
        )
    })?;
    let primary_repository_root = primary_repository_root.canonicalize()?;
    let primary_git_dir = primary_repository_root
        .join(".git")
        .canonicalize()
        .map_err(|_| unsupported_separate_git_dir_error(common_git_dir))?;
    if !primary_git_dir.is_dir() || primary_git_dir != common_git_dir {
        return Err(unsupported_separate_git_dir_error(common_git_dir));
    }
    Ok(primary_repository_root)
}

fn unsupported_separate_git_dir_error(common_git_dir: &Path) -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidInput,
        format!(
            "unsupported separate-Git-dir repository: common Git directory {} must be <primary repository root>/.git",
            common_git_dir.display()
        ),
    )
}

fn resolve_explicit_state_root(
    start: &Path,
    explicit_state_root: &Path,
    requirement: StateRootRequirement,
) -> io::Result<PathBuf> {
    let may_be_missing = requirement == StateRootRequirement::MayBeMissing;
    let absolute = absolute_path(start, explicit_state_root);
    if !may_be_missing || absolute.exists() {
        let canonical = absolute.canonicalize().map_err(|_| {
            io::Error::new(
                io::ErrorKind::NotFound,
                format!("{} does not exist", explicit_state_root.display()),
            )
        })?;
        if !canonical.is_dir() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("{} is not a directory", explicit_state_root.display()),
            ));
        }
        if !may_be_missing
            && (!canonical.join("agents").is_dir() || !canonical.join("tickets").is_dir())
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!(
                    "state directory {} must contain agents and tickets directories",
                    canonical.display()
                ),
            ));
        }
        return Ok(canonical);
    }
    Ok(normalize_absolute_path(&absolute))
}

fn home_directory() -> io::Result<PathBuf> {
    let home = env::var_os("HOME").ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "HOME must be set to an absolute path",
        )
    })?;
    let home = PathBuf::from(home);
    if !home.is_absolute() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "HOME must be set to an absolute path",
        ));
    }
    Ok(home)
}

fn state_root_for_primary(primary_repository_root: &Path, home: &Path) -> io::Result<PathBuf> {
    if !home.is_absolute() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "HOME must be set to an absolute path",
        ));
    }
    let relative_primary = primary_repository_root
        .strip_prefix(Path::new("/"))
        .map_err(|_| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                format!(
                    "primary repository root {} must be absolute",
                    primary_repository_root.display()
                ),
            )
        })?;
    Ok(home.join(".local/state/waap/data").join(relative_primary))
}

fn absolute_path(start: &Path, path: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        start.join(path)
    }
}

fn normalize_absolute_path(path: &Path) -> PathBuf {
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                normalized.pop();
            }
            Component::RootDir | Component::Prefix(_) | Component::Normal(_) => {
                normalized.push(component.as_os_str());
            }
        }
    }
    normalized
}

fn not_inside_git_repository_error() -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, "not inside a git repository")
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::Path;

    use tempfile::tempdir;

    use super::{
        resolve_project_context as resolve_context, state_root_for_primary, ProjectContext,
        StateRootRequirement,
    };
    use crate::test_git::{init_repo, isolate, run as git};

    fn resolve_project_context(
        start: &Path,
        explicit_state_root: Option<&Path>,
    ) -> std::io::Result<ProjectContext> {
        resolve_context(start, explicit_state_root, StateRootRequirement::Existing)
    }

    #[test]
    fn project_context_derives_state_from_canonical_primary_checkout() {
        let dir = tempdir().unwrap();
        init_repo(dir.path());
        let subdirectory = dir.path().join("source/nested");
        fs::create_dir_all(&subdirectory).unwrap();

        let context = resolve_project_context(&subdirectory, None).unwrap();
        let primary = dir.path().canonicalize().unwrap();
        let expected_state = state_root_for_primary(
            &primary,
            &std::path::PathBuf::from(std::env::var_os("HOME").unwrap()),
        )
        .unwrap();

        assert_eq!(context.invocation_worktree_root, primary);
        assert_eq!(
            context.primary_repository_root,
            dir.path().canonicalize().unwrap()
        );
        assert_eq!(
            context.common_git_dir,
            dir.path().join(".git").canonicalize().unwrap()
        );
        assert_eq!(context.state_root, expected_state);
    }

    #[test]
    fn linked_worktrees_share_primary_and_state_but_keep_invocation_root() {
        let dir = tempdir().unwrap();
        init_repo(dir.path());
        fs::write(dir.path().join("README"), "seed").unwrap();
        git(dir.path(), &["add", "."]);
        git(dir.path(), &["commit", "-q", "-m", "seed"]);
        let first = dir.path().join("first");
        let second = dir.path().join("second");
        git(
            dir.path(),
            &["worktree", "add", first.to_str().unwrap(), "-b", "first"],
        );
        git(
            dir.path(),
            &["worktree", "add", second.to_str().unwrap(), "-b", "second"],
        );

        let primary_context = resolve_project_context(dir.path(), None).unwrap();
        let first_context = resolve_project_context(&first, None).unwrap();
        let second_context = resolve_project_context(&second, None).unwrap();

        assert_eq!(
            first_context.primary_repository_root,
            primary_context.primary_repository_root
        );
        assert_eq!(
            second_context.primary_repository_root,
            primary_context.primary_repository_root
        );
        assert_eq!(first_context.common_git_dir, primary_context.common_git_dir);
        assert_eq!(
            second_context.common_git_dir,
            primary_context.common_git_dir
        );
        assert_eq!(first_context.state_root, primary_context.state_root);
        assert_eq!(second_context.state_root, primary_context.state_root);
        assert_eq!(
            first_context.invocation_worktree_root,
            first.canonicalize().unwrap()
        );
        assert_eq!(
            second_context.invocation_worktree_root,
            second.canonicalize().unwrap()
        );
    }

    #[cfg(unix)]
    #[test]
    fn symlinked_checkout_derives_the_primary_checkouts_state() {
        use std::os::unix::fs::symlink;

        let dir = tempdir().unwrap();
        init_repo(dir.path());
        let link = dir.path().with_extension("link");
        symlink(dir.path(), &link).unwrap();

        let direct = resolve_project_context(dir.path(), None).unwrap();
        let through_link = resolve_project_context(&link, None).unwrap();

        assert_eq!(through_link, direct);
    }

    #[test]
    fn explicit_state_root_is_not_derived_or_compared() {
        let dir = tempdir().unwrap();
        init_repo(dir.path());
        let state = tempdir().unwrap();
        fs::create_dir_all(state.path().join("agents")).unwrap();
        fs::create_dir_all(state.path().join("tickets")).unwrap();

        let context = resolve_project_context(dir.path(), Some(state.path())).unwrap();

        assert_eq!(context.state_root, state.path().canonicalize().unwrap());
        assert_eq!(
            context.invocation_worktree_root,
            dir.path().canonicalize().unwrap()
        );
    }

    #[test]
    fn explicit_state_root_requires_state_directories_except_for_init() {
        let dir = tempdir().unwrap();
        init_repo(dir.path());
        let missing = dir.path().join("new-state");

        let err = resolve_project_context(dir.path(), Some(&missing)).unwrap_err();
        assert!(err.to_string().contains("does not exist"));

        let context = resolve_context(
            dir.path(),
            Some(&missing),
            StateRootRequirement::MayBeMissing,
        )
        .unwrap();
        assert_eq!(context.state_root, missing);

        fs::create_dir(&missing).unwrap();
        let err = resolve_project_context(dir.path(), Some(&missing)).unwrap_err();
        assert!(err.to_string().contains("must contain agents and tickets"));
    }

    #[test]
    fn state_worktree_cannot_be_an_application_source() {
        let dir = tempdir().unwrap();
        init_repo(dir.path());
        fs::create_dir_all(dir.path().join("agents")).unwrap();
        fs::create_dir_all(dir.path().join("tickets")).unwrap();

        let context = resolve_project_context(dir.path(), Some(dir.path())).unwrap();

        let err = context.application_source_root().unwrap_err();
        assert!(err
            .to_string()
            .contains("cannot be used as an application source"));
    }

    #[test]
    fn state_root_derivation_appends_the_absolute_primary_path() {
        let root =
            state_root_for_primary(Path::new("/srv/projects/example"), Path::new("/home/test"))
                .unwrap();

        assert_eq!(
            root,
            Path::new("/home/test/.local/state/waap/data/srv/projects/example")
        );
    }

    #[test]
    fn state_root_derivation_requires_absolute_home_and_primary_paths() {
        let err = state_root_for_primary(Path::new("relative/repository"), Path::new("/home/test"))
            .unwrap_err();
        assert!(err.to_string().contains("must be absolute"));

        let err = state_root_for_primary(Path::new("/repository"), Path::new("relative-home"))
            .unwrap_err();
        assert!(err
            .to_string()
            .contains("HOME must be set to an absolute path"));
    }

    #[test]
    fn project_context_rejects_bare_repositories() {
        let dir = tempdir().unwrap();
        let bare = dir.path().join("bare");
        let mut command = std::process::Command::new("git");
        isolate(&mut command);
        let output = command
            .args(["init", "--bare", bare.to_str().unwrap()])
            .output()
            .unwrap();
        assert!(output.status.success());

        let err = resolve_project_context(&bare, None).unwrap_err();
        assert!(err.to_string().contains("unsupported bare repository"));
    }

    #[test]
    fn project_context_rejects_separate_git_dir_layouts() {
        let dir = tempdir().unwrap();
        let worktree = dir.path().join("worktree");
        let git_dir = dir.path().join("separate-git-dir");
        fs::create_dir_all(&worktree).unwrap();
        fs::create_dir_all(&git_dir).unwrap();
        fs::write(
            worktree.join(".git"),
            format!("gitdir: {}\n", git_dir.display()),
        )
        .unwrap();

        let err = resolve_project_context(&worktree, None).unwrap_err();
        assert!(err
            .to_string()
            .contains("unsupported separate-Git-dir repository"));
    }
}
