use std::io;
use std::path::{Path, PathBuf};

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

/// Resolve the waap project root, either by validating an explicit `--waap-root` or by walking
/// up from `start` to the nearest `.waap/`, bounded by the git root.
///
/// When `explicit_waap_root` is `None`, the git root is resolved from `start` first, so "not
/// inside a git repository" always takes precedence over "no waap project found".
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
            if find_git_root(&canonical).is_none() {
                return Err(not_inside_git_repository_error());
            }
            if !canonical.join(".waap").is_dir() {
                return Err(io::Error::new(
                    io::ErrorKind::NotFound,
                    format!(
                        "no waap project at {}; run 'waap init' or omit --waap-root",
                        explicit.display()
                    ),
                ));
            }
            Ok(canonical)
        }
        None => {
            let canonical_start = start.canonicalize()?;
            let git_root =
                find_git_root(&canonical_start).ok_or_else(not_inside_git_repository_error)?;
            find_waap_root(&canonical_start, &git_root).ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::NotFound,
                    "no waap project found; run 'waap init'",
                )
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::io;
    use std::path::Path;
    use std::process::Command;

    use tempfile::{tempdir, TempDir};

    use super::resolve_waap_root;

    /// A tempdir guaranteed to have no `.git` anywhere in its ancestry.
    ///
    /// The default tempdir base (`/tmp`) is shared scratch space that can carry stray `.git`
    /// directories left behind by unrelated tooling, which would falsely satisfy the git-root
    /// walk in tests that assert "not inside a git repository". `/var/tmp` is a separate,
    /// unrelated temp filesystem.
    fn tempdir_outside_any_git_repo() -> TempDir {
        tempfile::Builder::new().tempdir_in("/var/tmp").unwrap()
    }

    fn init_repo(root: &Path) {
        let status = Command::new("git")
            .current_dir(root)
            .args(["init", "-q"])
            .status()
            .unwrap();
        assert!(status.success());
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
    fn errors_when_waap_only_exists_above_git_root() {
        let dir = tempdir().unwrap();
        fs::create_dir_all(dir.path().join(".waap")).unwrap();
        let repo = dir.path().join("repo");
        fs::create_dir_all(&repo).unwrap();
        init_repo(&repo);

        let err = resolve_waap_root(&repo, None).unwrap_err();

        assert_eq!(err.kind(), io::ErrorKind::NotFound);
        assert!(err.to_string().contains("waap init"));
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
        let status = Command::new("git")
            .current_dir(dir.path())
            .args(["add", "."])
            .status()
            .unwrap();
        assert!(status.success());
        let status = Command::new("git")
            .current_dir(dir.path())
            .args([
                "-c",
                "user.name=Test",
                "-c",
                "user.email=test@example.com",
                "commit",
                "-q",
                "-m",
                "seed",
            ])
            .status()
            .unwrap();
        assert!(status.success());

        let worktree = dir.path().join("worktree");
        let status = Command::new("git")
            .current_dir(dir.path())
            .args([
                "worktree",
                "add",
                worktree.to_str().unwrap(),
                "-b",
                "feature",
            ])
            .status()
            .unwrap();
        assert!(status.success());
        assert!(worktree.join(".git").is_file());
        fs::create_dir_all(worktree.join(".waap")).unwrap();

        let root = resolve_waap_root(&worktree, None).unwrap();

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

        let err = resolve_waap_root(dir.path(), Some(&sub)).unwrap_err();

        assert_eq!(err.kind(), io::ErrorKind::NotFound);
        assert!(err.to_string().contains("no waap project at"));
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
    fn explicit_waap_root_errors_when_it_lacks_a_direct_waap_dir() {
        let dir = tempdir().unwrap();
        init_repo(dir.path());

        let err = resolve_waap_root(dir.path(), Some(dir.path())).unwrap_err();

        assert_eq!(err.kind(), io::ErrorKind::NotFound);
        assert!(err.to_string().contains("no waap project at"));
        assert!(err
            .to_string()
            .contains("run 'waap init' or omit --waap-root"));
    }

    #[test]
    fn explicit_waap_root_resolves_when_valid() {
        let dir = tempdir().unwrap();
        init_repo(dir.path());
        fs::create_dir_all(dir.path().join(".waap")).unwrap();

        let root = resolve_waap_root(dir.path(), Some(dir.path())).unwrap();

        assert_eq!(root, dir.path().canonicalize().unwrap());
    }
}
