use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::Path;

use crate::git::run_git;

/// Serializes a state read/modify/commit transaction across waap processes.
pub(crate) struct StateLock {
    _file: File,
}

impl StateLock {
    pub(crate) fn acquire_for_read(state_root: &Path) -> io::Result<Option<Self>> {
        if state_root.is_dir() && crate::root::find_git_root(state_root).is_ok() {
            Self::acquire(state_root).map(Some)
        } else {
            Ok(None)
        }
    }

    pub(crate) fn acquire(state_root: &Path) -> io::Result<Self> {
        let output = run_git(
            state_root,
            &[
                "rev-parse".into(),
                "--path-format=absolute".into(),
                "--git-dir".into(),
            ],
        )?;
        let git_dir = String::from_utf8_lossy(&output.stdout);
        let path = Path::new(git_dir.trim()).join("waap-state.lock");
        // Keep the inode after unlocking: unlinking it would let later callers lock a different file.
        let file = OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(&path)?;
        file.lock().map_err(|error| {
            io::Error::new(
                error.kind(),
                format!("failed to lock {}: {error}", path.display()),
            )
        })?;
        Ok(Self { _file: file })
    }
}

pub(crate) fn write_record_atomically(path: &Path, contents: &str) -> io::Result<()> {
    let parent = path.parent().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "record path must have a parent directory",
        )
    })?;
    fs::create_dir_all(parent)?;
    let mut builder = tempfile::Builder::new();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        builder.permissions(fs::Permissions::from_mode(0o666));
    }
    let mut temporary = builder.tempfile_in(parent)?;
    if let Ok(metadata) = fs::metadata(path) {
        temporary
            .as_file()
            .set_permissions(metadata.permissions())?;
    }
    temporary.write_all(contents.as_bytes())?;
    temporary.as_file().sync_all()?;
    temporary.persist(path).map_err(|error| {
        io::Error::new(
            error.error.kind(),
            format!("failed to replace {}: {}", path.display(), error.error),
        )
    })?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_git::init_repo_with_commit;

    #[test]
    fn state_lock_excludes_other_handles_and_releases_on_drop() {
        let dir = tempfile::tempdir().unwrap();
        init_repo_with_commit(dir.path());
        let lock = StateLock::acquire(dir.path()).unwrap();
        let contender = OpenOptions::new()
            .write(true)
            .open(dir.path().join(".git/waap-state.lock"))
            .unwrap();
        assert!(matches!(
            contender.try_lock(),
            Err(std::fs::TryLockError::WouldBlock)
        ));
        drop(lock);
        contender.try_lock().unwrap();
    }

    #[test]
    fn atomic_write_replaces_whole_record_without_leaving_temporary_files() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("record.md");
        write_record_atomically(&path, "original").unwrap();
        write_record_atomically(&path, "replacement").unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), "replacement");
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 1);
    }

    #[test]
    fn failed_publication_preserves_destination_and_cleans_temporary_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("record.md");
        fs::create_dir(&path).unwrap();
        fs::write(path.join("preserved"), "original").unwrap();
        assert!(write_record_atomically(&path, "replacement").is_err());
        assert_eq!(
            fs::read_to_string(path.join("preserved")).unwrap(),
            "original"
        );
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 1);
    }

    #[cfg(unix)]
    #[test]
    fn new_record_permissions_follow_normal_file_creation() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let reference = dir.path().join("reference");
        let record = dir.path().join("record.md");
        fs::write(&reference, "reference").unwrap();
        write_record_atomically(&record, "record").unwrap();
        assert_eq!(
            fs::metadata(record).unwrap().permissions().mode() & 0o777,
            fs::metadata(reference).unwrap().permissions().mode() & 0o777
        );
    }

    #[cfg(unix)]
    #[test]
    fn atomic_write_preserves_existing_permissions() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("record.md");
        fs::write(&path, "original").unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o640)).unwrap();
        write_record_atomically(&path, "replacement").unwrap();
        assert_eq!(
            fs::metadata(path).unwrap().permissions().mode() & 0o777,
            0o640
        );
    }
}
