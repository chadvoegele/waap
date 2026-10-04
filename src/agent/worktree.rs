use std::io;
use std::path::{Path, PathBuf};

use crate::git::{create_worktree, remove_worktree};

pub(super) fn agent_worktree_dir(agent_id: &str) -> PathBuf {
    Path::new("worktrees").join(agent_id)
}

pub(super) struct AgentWorktree {
    repository_root: PathBuf,
    relative_path: PathBuf,
    worktree_dir: PathBuf,
    cleanup_pending: bool,
}

impl AgentWorktree {
    pub(super) fn create(repository_root: &Path, agent_id: &str) -> io::Result<Self> {
        let relative_path = agent_worktree_dir(agent_id);
        let worktree_dir = create_worktree(repository_root, agent_id, &relative_path)?;
        Ok(Self {
            repository_root: repository_root.to_path_buf(),
            relative_path,
            worktree_dir,
            cleanup_pending: true,
        })
    }

    pub(super) fn dir(&self) -> &Path {
        &self.worktree_dir
    }

    pub(super) fn cleanup(&mut self) -> io::Result<()> {
        if !self.cleanup_pending {
            return Ok(());
        }
        remove_worktree(&self.repository_root, &self.relative_path)?;
        self.cleanup_pending = false;
        Ok(())
    }
}

pub(super) fn collapse_errors<T>(
    run_result: io::Result<T>,
    cleanup_result: io::Result<()>,
) -> io::Result<T> {
    match (run_result, cleanup_result) {
        (Ok(value), Ok(())) => Ok(value),
        (Ok(_), Err(cleanup_error)) => Err(cleanup_error),
        (Err(run_error), Ok(())) => Err(run_error),
        (Err(run_error), Err(cleanup_error)) => Err(io::Error::new(
            run_error.kind(),
            format!("{run_error}; worktree cleanup also failed: {cleanup_error}"),
        )),
    }
}

impl Drop for AgentWorktree {
    fn drop(&mut self) {
        if let Err(error) = self.cleanup() {
            log::error!(
                "failed to clean up agent worktree {}: {error}",
                self.worktree_dir.display()
            );
        }
    }
}
