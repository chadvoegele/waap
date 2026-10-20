use std::fs::{self, DirBuilder};
use std::io;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::{DirBuilderExt, MetadataExt, PermissionsExt};
use std::path::{Component, Path, PathBuf};

use crate::ids::is_record_id;
use crate::root::ControlRoot;

pub(in crate::agent) fn socket_path(
    control_root: &ControlRoot,
    agent_id: &str,
) -> io::Result<PathBuf> {
    if !is_record_id(agent_id, "aa-") {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "invalid agent ID",
        ));
    }
    require_private_directory(control_root.runtime_dir())?;
    let path = control_root
        .path()
        .join("agents")
        .join(format!("{agent_id}.sock"));
    let address: libc::sockaddr_un = unsafe { std::mem::zeroed() };
    if path.as_os_str().as_bytes().len() >= address.sun_path.len() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!(
                "control socket path exceeds the {}-byte platform limit: {}; shorten XDG_RUNTIME_DIR or the repository path",
                address.sun_path.len() - 1,
                path.display()
            ),
        ));
    }
    Ok(path)
}

pub(super) fn prepare_parent(path: &Path, control_root: &ControlRoot) -> io::Result<()> {
    let runtime_dir = control_root.runtime_dir();
    let mut directory = runtime_dir.to_path_buf();
    for component in path
        .parent()
        .unwrap()
        .strip_prefix(runtime_dir)
        .unwrap()
        .components()
    {
        let Component::Normal(name) = component else {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "invalid runtime path",
            ));
        };
        directory.push(name);
        match DirBuilder::new().mode(0o700).create(&directory) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(error),
        }
        require_private_directory(&directory)?;
    }
    Ok(())
}

fn require_private_directory(path: &Path) -> io::Result<()> {
    let metadata = fs::symlink_metadata(path).map_err(|error| {
        io::Error::new(
            error.kind(),
            format!("invalid runtime directory {}: {error}", path.display()),
        )
    })?;
    validate_directory(
        path,
        metadata.is_dir(),
        metadata.uid(),
        metadata.permissions().mode(),
    )
}

fn validate_directory(path: &Path, is_directory: bool, uid: u32, mode: u32) -> io::Result<()> {
    if !is_directory || uid != unsafe { libc::geteuid() } || mode & 0o077 != 0 {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            format!(
                "runtime directory {} must be owned by the current user and private (0700)",
                path.display()
            ),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::root::find_control_root_in;
    use crate::test_git::{init_repo_with_commit, run as git};
    use std::ffi::OsStr;

    fn resolve_socket(
        repository_root: &Path,
        agent_id: &str,
        runtime_dir: Option<&OsStr>,
    ) -> io::Result<PathBuf> {
        socket_path(
            &find_control_root_in(repository_root, runtime_dir)?,
            agent_id,
        )
    }

    #[test]
    fn injected_control_root_does_not_resolve_repository_again() {
        let runtime = tempfile::tempdir().unwrap();
        fs::set_permissions(runtime.path(), fs::Permissions::from_mode(0o700)).unwrap();
        let repo = tempfile::tempdir().unwrap();
        init_repo_with_commit(repo.path());
        let control_root =
            find_control_root_in(repo.path(), Some(runtime.path().as_os_str())).unwrap();
        fs::remove_dir_all(repo.path().join(".git")).unwrap();
        assert_eq!(
            socket_path(&control_root, "aa-1").unwrap(),
            control_root.path().join("agents/aa-1.sock")
        );
        fs::set_permissions(runtime.path(), fs::Permissions::from_mode(0o755)).unwrap();
        assert!(socket_path(&control_root, "aa-1")
            .unwrap_err()
            .to_string()
            .contains("private (0700)"));
    }

    #[test]
    fn socket_identity_is_symmetric_across_worktrees_and_isolates_repositories() {
        let runtime = tempfile::tempdir().unwrap();
        fs::set_permissions(runtime.path(), fs::Permissions::from_mode(0o700)).unwrap();
        let root = tempfile::tempdir().unwrap();
        let repo = root.path().join("a");
        let other = root.path().join("b");
        fs::create_dir(&repo).unwrap();
        fs::create_dir(&other).unwrap();
        init_repo_with_commit(&repo);
        init_repo_with_commit(&other);
        let linked = root.path().join("linked");
        git(
            &repo,
            &["worktree", "add", linked.to_str().unwrap(), "-b", "linked"],
        );
        let resolve =
            |repo: &Path| resolve_socket(repo, "aa-1", Some(runtime.path().as_os_str())).unwrap();
        let expected = runtime
            .path()
            .join("waap/control")
            .join(repo.canonicalize().unwrap().strip_prefix("/").unwrap())
            .join("agents/aa-1.sock");
        assert_eq!(resolve(&repo), expected);
        assert_eq!(resolve(&linked), expected);
        assert_ne!(resolve(&other), expected);
    }

    #[test]
    fn invalid_runtime_and_paths_fail_without_fallback() {
        let runtime = tempfile::tempdir().unwrap();
        let repo = tempfile::tempdir().unwrap();
        init_repo_with_commit(repo.path());
        for value in [None, Some(OsStr::new("")), Some(OsStr::new("relative"))] {
            assert!(resolve_socket(repo.path(), "aa-1", value).is_err());
        }
        assert!(
            validate_directory(runtime.path(), true, unsafe { libc::geteuid() } + 1, 0o700)
                .is_err()
        );
        assert!(
            validate_directory(runtime.path(), false, unsafe { libc::geteuid() }, 0o700).is_err()
        );
        fs::set_permissions(runtime.path(), fs::Permissions::from_mode(0o750)).unwrap();
        assert!(resolve_socket(repo.path(), "aa-1", Some(runtime.path().as_os_str())).is_err());
        fs::set_permissions(runtime.path(), fs::Permissions::from_mode(0o700)).unwrap();
        let error = resolve_socket(
            repo.path(),
            &format!("aa-{}", "a".repeat(63)),
            Some(runtime.path().as_os_str()),
        )
        .unwrap_err();
        assert!(error.to_string().contains("platform limit"));
        assert!(error.to_string().contains("shorten XDG_RUNTIME_DIR"));
        assert!(!runtime.path().join("waap").exists());
        assert!(
            resolve_socket(repo.path(), "../escape", Some(runtime.path().as_os_str())).is_err()
        );
        let link = repo.path().join("runtime-link");
        std::os::unix::fs::symlink(runtime.path(), &link).unwrap();
        assert!(resolve_socket(repo.path(), "aa-1", Some(link.as_os_str())).is_err());
    }
}
