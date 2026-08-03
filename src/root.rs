#![allow(dead_code)] // Context resolution is introduced before command dispatch adopts central state.

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

/// Resolve a context for an existing state directory. An explicit state root
/// must contain the state directories directly.
pub(crate) fn resolve_project_context(
    start: &Path,
    explicit_state_root: Option<&Path>,
) -> io::Result<ProjectContext> {
    resolve_project_context_inner(start, explicit_state_root, false)
}

/// Resolve a context for `waap init`. Unlike regular explicit resolution, an
/// explicit state-root target may not exist yet.
pub(crate) fn resolve_init_project_context(
    start: &Path,
    explicit_state_root: Option<&Path>,
) -> io::Result<ProjectContext> {
    resolve_project_context_inner(start, explicit_state_root, true)
}

fn resolve_project_context_inner(
    start: &Path,
    explicit_state_root: Option<&Path>,
    allow_missing_explicit_state_root: bool,
) -> io::Result<ProjectContext> {
    let canonical_start = start.canonicalize()?;
    let invocation_worktree_root = find_invocation_worktree_root(&canonical_start)?;
    let common_git_dir = resolve_common_git_dir(&invocation_worktree_root)?;
    let primary_repository_root = resolve_primary_repository_root(&common_git_dir)?;
    let state_root = match explicit_state_root {
        Some(path) => {
            resolve_explicit_state_root(&canonical_start, path, allow_missing_explicit_state_root)?
        }
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
    allow_missing: bool,
) -> io::Result<PathBuf> {
    let absolute = absolute_path(start, explicit_state_root);
    if !allow_missing || absolute.exists() {
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
        if !allow_missing
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

/// Walk up from `start` to the nearest ancestor containing a `.git` entry (file or directory).
///
/// Does not shell out to `git rev-parse --show-toplevel`: from a linked worktree that would
/// return the main repository's toplevel, which is the wrong boundary for an agent running in
/// its own `worktrees/<id>` checkout.
fn find_git_root(start: &Path) -> Option<PathBuf> {
    let mut current = start;
    loop {
        if current.join(".git").exists() {
            return Some(current.to_path_buf());
        }
        current = current.parent()?;
    }
}

/// Walk up from `start` to the nearest ancestor containing `.waap/`, never searching above
/// `git_root`.
fn find_waap_root(start: &Path, git_root: &Path) -> Option<PathBuf> {
    let mut current = start;
    loop {
        if current.join(".waap").is_dir() {
            return Some(current.to_path_buf());
        }
        if current == git_root {
            return None;
        }
        current = current.parent()?;
    }
}

fn not_inside_git_repository_error() -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, "not inside a git repository")
}

/// Resolve the waap project root from an explicit `--waap-root` or by walking up from `start` to
/// the nearest `.waap/`, bounded by and falling back to the git root.
pub(crate) fn resolve_waap_root(
    start: &Path,
    explicit_waap_root: Option<&Path>,
) -> io::Result<PathBuf> {
    match explicit_waap_root {
        Some(explicit) => {
            let canonical = explicit.canonicalize().map_err(|_| {
                io::Error::new(
                    io::ErrorKind::NotFound,
                    format!("{} does not exist", explicit.display()),
                )
            })?;
            if !canonical.is_dir() {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    format!("{} is not a directory", explicit.display()),
                ));
            }
            if find_git_root(&canonical).is_none() {
                return Err(not_inside_git_repository_error());
            }
            Ok(canonical)
        }
        None => {
            let canonical_start = start.canonicalize()?;
            let git_root =
                find_git_root(&canonical_start).ok_or_else(not_inside_git_repository_error)?;
            Ok(find_waap_root(&canonical_start, &git_root).unwrap_or(git_root))
        }
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::io;
    use std::path::Path;

    use tempfile::{tempdir, TempDir};

    use super::{
        resolve_init_project_context, resolve_project_context, resolve_waap_root,
        state_root_for_primary,
    };
    use crate::test_git::{init_repo, isolate, run as git};

    /// A tempdir guaranteed to have no `.git` anywhere in its ancestry.
    ///
    /// The default tempdir base (`/tmp`) can carry stray `.git` directories left by unrelated
    /// tooling, which would falsely satisfy the git-root walk. `/dev/shm` is separate,
    /// memory-backed scratch space outside the project's Git ancestry.
    fn tempdir_outside_any_git_repo() -> TempDir {
        tempfile::Builder::new()
            .tempdir_in("/dev/shm")
            .expect("failed to create git-isolated tempdir in /dev/shm")
    }

    #[test]
    fn resolves_cwd_when_it_is_the_project_root() {
        let dir = tempdir().unwrap();
        init_repo(dir.path());
        fs::create_dir_all(dir.path().join(".waap")).unwrap();

        let root = resolve_waap_root(dir.path(), None).unwrap();

        assert_eq!(root, dir.path().canonicalize().unwrap());
    }

    #[test]
    fn resolves_nearest_ancestor_waap_from_subdirectory() {
        let dir = tempdir().unwrap();
        init_repo(dir.path());
        fs::create_dir_all(dir.path().join(".waap")).unwrap();
        let sub = dir.path().join("a/b/c");
        fs::create_dir_all(&sub).unwrap();

        let root = resolve_waap_root(&sub, None).unwrap();

        assert_eq!(root, dir.path().canonicalize().unwrap());
    }

    #[test]
    fn resolves_nearest_of_two_nested_projects_from_outer() {
        let dir = tempdir().unwrap();
        init_repo(dir.path());
        let lab = dir.path().join("lab");
        let wiki = lab.join("wiki");
        fs::create_dir_all(lab.join(".waap")).unwrap();
        fs::create_dir_all(wiki.join(".waap")).unwrap();

        let root = resolve_waap_root(&lab, None).unwrap();

        assert_eq!(root, lab.canonicalize().unwrap());
    }

    #[test]
    fn resolves_nearest_of_two_nested_projects_from_inner() {
        let dir = tempdir().unwrap();
        init_repo(dir.path());
        let lab = dir.path().join("lab");
        let wiki = lab.join("wiki");
        fs::create_dir_all(lab.join(".waap")).unwrap();
        fs::create_dir_all(wiki.join(".waap")).unwrap();

        let root = resolve_waap_root(&wiki, None).unwrap();

        assert_eq!(root, wiki.canonicalize().unwrap());
    }

    #[test]
    fn falls_back_to_git_root_without_searching_above_it() {
        let dir = tempdir().unwrap();
        fs::create_dir_all(dir.path().join(".waap")).unwrap();
        let repo = dir.path().join("repo");
        fs::create_dir_all(&repo).unwrap();
        init_repo(&repo);

        let root = resolve_waap_root(&repo, None).unwrap();

        assert_eq!(root, repo.canonicalize().unwrap());
    }

    #[test]
    fn errors_when_not_inside_a_git_repository() {
        let dir = tempdir_outside_any_git_repo();

        let err = resolve_waap_root(dir.path(), None).unwrap_err();

        assert_eq!(err.kind(), io::ErrorKind::InvalidInput);
        assert!(err.to_string().contains("not inside a git repository"));
    }

    #[test]
    fn not_in_git_repo_error_precedes_no_waap_project_error() {
        let dir = tempdir_outside_any_git_repo();
        // No .git anywhere, but a .waap is present: the git-root check must still fire first.
        fs::create_dir_all(dir.path().join(".waap")).unwrap();

        let err = resolve_waap_root(dir.path(), None).unwrap_err();

        assert_eq!(err.kind(), io::ErrorKind::InvalidInput);
        assert!(err.to_string().contains("not inside a git repository"));
    }

    #[test]
    fn resolves_linked_worktree_with_its_own_waap() {
        let dir = tempdir().unwrap();
        init_repo(dir.path());
        fs::write(dir.path().join("README"), "seed").unwrap();
        git(dir.path(), &["add", "."]);
        git(dir.path(), &["commit", "-q", "-m", "seed"]);

        let worktree = dir.path().join("worktree");
        git(
            dir.path(),
            &[
                "worktree",
                "add",
                worktree.to_str().unwrap(),
                "-b",
                "feature",
            ],
        );
        assert!(worktree.join(".git").is_file());
        fs::create_dir_all(worktree.join(".waap")).unwrap();

        let root = resolve_waap_root(&worktree, None).unwrap();

        assert_eq!(root, worktree.canonicalize().unwrap());
    }

    #[test]
    fn falls_back_to_linked_worktree_root_without_waap() {
        let dir = tempdir().unwrap();
        init_repo(dir.path());
        fs::write(dir.path().join("README"), "seed").unwrap();
        git(dir.path(), &["add", "."]);
        git(dir.path(), &["commit", "-q", "-m", "seed"]);

        let worktree = dir.path().join("worktree");
        git(
            dir.path(),
            &[
                "worktree",
                "add",
                worktree.to_str().unwrap(),
                "-b",
                "feature",
            ],
        );
        let sub = worktree.join("deep/nested");
        fs::create_dir_all(&sub).unwrap();

        let root = resolve_waap_root(&sub, None).unwrap();

        assert!(worktree.join(".git").is_file());
        assert_eq!(root, worktree.canonicalize().unwrap());
    }

    #[test]
    fn resolves_git_root_itself_when_only_waap_there() {
        let dir = tempdir().unwrap();
        init_repo(dir.path());
        fs::create_dir_all(dir.path().join(".waap")).unwrap();
        let sub = dir.path().join("deep/nested/dir");
        fs::create_dir_all(&sub).unwrap();

        let root = resolve_waap_root(&sub, None).unwrap();

        assert_eq!(root, dir.path().canonicalize().unwrap());
    }

    #[test]
    fn explicit_waap_root_is_used_exactly_without_walking() {
        let dir = tempdir().unwrap();
        init_repo(dir.path());
        fs::create_dir_all(dir.path().join(".waap")).unwrap();
        let sub = dir.path().join("sub");
        fs::create_dir_all(&sub).unwrap();

        let root = resolve_waap_root(dir.path(), Some(&sub)).unwrap();

        assert_eq!(root, sub.canonicalize().unwrap());
    }

    #[test]
    fn explicit_waap_root_errors_when_it_does_not_exist() {
        let dir = tempdir().unwrap();
        let missing = dir.path().join("missing");

        let err = resolve_waap_root(dir.path(), Some(&missing)).unwrap_err();

        assert_eq!(err.kind(), io::ErrorKind::NotFound);
        assert!(err.to_string().contains("does not exist"));
    }

    #[test]
    fn explicit_waap_root_errors_when_not_inside_git_repository() {
        let dir = tempdir_outside_any_git_repo();
        fs::create_dir_all(dir.path().join(".waap")).unwrap();

        let err = resolve_waap_root(dir.path(), Some(dir.path())).unwrap_err();

        assert_eq!(err.kind(), io::ErrorKind::InvalidInput);
        assert!(err.to_string().contains("not inside a git repository"));
    }

    #[test]
    fn explicit_waap_root_errors_when_it_is_not_a_directory() {
        let dir = tempdir().unwrap();
        init_repo(dir.path());
        let file = dir.path().join("file");
        fs::write(&file, "contents").unwrap();

        let err = resolve_waap_root(dir.path(), Some(&file)).unwrap_err();

        assert_eq!(err.kind(), io::ErrorKind::InvalidInput);
        assert!(err.to_string().contains("is not a directory"));
    }

    #[test]
    fn explicit_waap_root_resolves_when_valid() {
        let dir = tempdir().unwrap();
        init_repo(dir.path());
        fs::create_dir_all(dir.path().join(".waap")).unwrap();

        let root = resolve_waap_root(dir.path(), Some(dir.path())).unwrap();

        assert_eq!(root, dir.path().canonicalize().unwrap());
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

        let context = resolve_init_project_context(dir.path(), Some(&missing)).unwrap();
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
