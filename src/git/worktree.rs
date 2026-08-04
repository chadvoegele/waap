use std::io;
use std::path::{Path, PathBuf};

use super::command::run;

pub(crate) fn create(
    repository_root: &Path,
    branch: &str,
    relative_path: &Path,
) -> io::Result<PathBuf> {
    run(
        repository_root,
        &[
            "worktree".into(),
            "add".into(),
            "-b".into(),
            branch.into(),
            relative_path.as_os_str().to_os_string(),
        ],
    )?;
    repository_root.join(relative_path).canonicalize()
}

pub(crate) fn remove(repository_root: &Path, relative_path: &Path) -> io::Result<()> {
    run(
        repository_root,
        &[
            "worktree".into(),
            "remove".into(),
            "--force".into(),
            relative_path.as_os_str().to_os_string(),
        ],
    )?;
    Ok(())
}
