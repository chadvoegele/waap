use std::ffi::OsString;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use super::command::{
    args as git_args, command_error, commit_paths, output as git_output, run as run_git,
};
use super::{STATE_BRANCH, STATE_LOCK_FILE};

/// A waap state checkout and the application checkout associated with it.
///
/// Commands use this type instead of passing several loosely related roots.
#[derive(Clone, Debug)]
pub(crate) struct StateStore {
    pub(crate) state_root: PathBuf,
    pub(crate) source_root: PathBuf,
    common_git_dir: PathBuf,
    layout: StateLayout,
}

#[derive(Clone, Copy, Debug)]
enum StateLayout {
    Central,
    #[cfg(test)]
    Legacy,
}

impl StateStore {
    #[cfg(test)]
    pub(crate) fn legacy(repository_root: &Path) -> io::Result<Self> {
        let repository_root = repository_root.canonicalize()?;
        let common_git_dir = common_git_dir(&repository_root)?;
        Ok(Self {
            source_root: repository_root.clone(),
            state_root: repository_root,
            common_git_dir,
            layout: StateLayout::Legacy,
        })
    }

    pub(crate) fn from_project_context(context: &crate::root::ProjectContext) -> Self {
        Self {
            state_root: context.state_root.clone(),
            source_root: context.invocation_worktree_root.clone(),
            common_git_dir: context.common_git_dir.clone(),
            layout: StateLayout::Central,
        }
    }

    #[cfg(test)]
    pub(crate) fn central(
        state_root: PathBuf,
        source_root: PathBuf,
        common_git_dir: PathBuf,
    ) -> Self {
        Self {
            state_root,
            source_root,
            common_git_dir,
            layout: StateLayout::Central,
        }
    }

    fn validator(&self) -> fn(&Path) -> Vec<String> {
        match self.layout {
            StateLayout::Central => crate::check::check_state,
            #[cfg(test)]
            StateLayout::Legacy => crate::check::check_waap,
        }
    }

    fn requires_state_branch(&self) -> bool {
        matches!(self.layout, StateLayout::Central)
    }
}

/// A serialized state mutation. It owns the per-repository lock from
/// validation through commit. Call `snapshot_path` before changing each file;
/// dropping an unfinished transaction restores those files and their index
/// entries to HEAD.
pub(crate) struct StateTransaction {
    store: StateStore,
    lock: StateLock,
    snapshots: Vec<PathSnapshot>,
    validate: fn(&Path) -> Vec<String>,
    finished: bool,
}

impl StateTransaction {
    pub(crate) fn begin(store: StateStore) -> io::Result<Self> {
        let validate = store.validator();
        Self::begin_with_validator(store, validate)
    }

    pub(crate) fn begin_with_validator(
        store: StateStore,
        validate: fn(&Path) -> Vec<String>,
    ) -> io::Result<Self> {
        let lock = StateLock::acquire(&store.common_git_dir)?;
        if store.requires_state_branch() {
            ensure_state_branch(&store.state_root)?;
        }
        validate_state(&store.state_root, validate)?;
        Ok(Self {
            store,
            lock,
            snapshots: Vec::new(),
            validate,
            finished: false,
        })
    }

    pub(crate) fn state_root(&self) -> &Path {
        &self.store.state_root
    }

    pub(crate) fn snapshot_path(&mut self, path: &Path) -> io::Result<()> {
        if !path.starts_with(&self.store.state_root) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!(
                    "state mutation path {} is outside state directory {}",
                    path.display(),
                    self.store.state_root.display()
                ),
            ));
        }
        if self.snapshots.iter().any(|snapshot| snapshot.path == path) {
            return Ok(());
        }
        ensure_path_unstaged(&self.store.state_root, path)?;
        self.snapshots.push(PathSnapshot {
            path: path.to_path_buf(),
            contents: if path.exists() {
                Some(fs::read(path)?)
            } else {
                None
            },
            missing_parent_dirs: missing_parent_dirs(path, &self.store.state_root),
        });
        Ok(())
    }

    pub(crate) fn validate(&self, validate: impl Fn(&Path) -> Vec<String>) -> io::Result<()> {
        validate_state(&self.store.state_root, validate)
    }

    pub(crate) fn commit(mut self, paths: &[&Path], message: &str) -> io::Result<String> {
        if paths.is_empty() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "no paths to commit",
            ));
        }
        for path in paths {
            if !self.snapshots.iter().any(|snapshot| snapshot.path == *path) {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    format!("state mutation path {} was not snapshotted", path.display()),
                ));
            }
        }
        if let Err(error) = self.validate(self.validate) {
            return Err(self.fail(error));
        }
        match commit_paths(&self.store.state_root, paths, message) {
            Ok(commit) => {
                self.finished = true;
                Ok(commit)
            }
            Err(error) => Err(self.fail(error)),
        }
    }

    fn fail(&mut self, error: io::Error) -> io::Error {
        self.finished = true;
        match self.restore() {
            Ok(()) => error,
            Err(restore_error) => io::Error::new(
                error.kind(),
                format!("{error}; failed to roll back waap state: {restore_error}"),
            ),
        }
    }

    fn restore(&self) -> io::Result<()> {
        let paths: Vec<&Path> = self
            .snapshots
            .iter()
            .map(|snapshot| snapshot.path.as_path())
            .collect();
        if !paths.is_empty() {
            let mut reset_args: Vec<OsString> = vec!["reset".into(), "--".into()];
            reset_args.extend(paths.iter().map(|path| path.as_os_str().to_os_string()));
            run_git(&self.store.state_root, &reset_args)?;
        }
        for snapshot in &self.snapshots {
            match &snapshot.contents {
                Some(contents) => {
                    if let Some(parent) = snapshot.path.parent() {
                        fs::create_dir_all(parent)?;
                    }
                    fs::write(&snapshot.path, contents)?;
                }
                None if snapshot.path.exists() => fs::remove_file(&snapshot.path)?,
                None => {}
            }
        }
        for directory in self
            .snapshots
            .iter()
            .flat_map(|snapshot| &snapshot.missing_parent_dirs)
        {
            match fs::remove_dir(directory) {
                Ok(()) => {}
                Err(error)
                    if matches!(
                        error.kind(),
                        io::ErrorKind::NotFound | io::ErrorKind::DirectoryNotEmpty
                    ) => {}
                Err(error) => return Err(error),
            }
        }
        Ok(())
    }
}

fn missing_parent_dirs(path: &Path, state_root: &Path) -> Vec<PathBuf> {
    let mut directories = Vec::new();
    let mut current = path.parent();
    while let Some(directory) = current {
        if directory == state_root || directory.exists() {
            break;
        }
        directories.push(directory.to_path_buf());
        current = directory.parent();
    }
    directories
}

impl Drop for StateTransaction {
    fn drop(&mut self) {
        if !self.finished {
            if let Err(error) = self.restore() {
                log::error!("failed to roll back waap state transaction: {error}");
            }
        }
        let _ = &self.lock;
    }
}

struct PathSnapshot {
    path: PathBuf,
    contents: Option<Vec<u8>>,
    missing_parent_dirs: Vec<PathBuf>,
}

struct StateLock {
    path: PathBuf,
}

impl StateLock {
    fn acquire(common_git_dir: &Path) -> io::Result<Self> {
        let path = common_git_dir.join(STATE_LOCK_FILE);
        match fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
        {
            Ok(mut file) => {
                writeln!(file, "{}", std::process::id())?;
                Ok(Self { path })
            }
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => Err(io::Error::new(
                io::ErrorKind::WouldBlock,
                format!(
                    "waap state transaction is already running ({}); retry after it finishes",
                    path.display()
                ),
            )),
            Err(error) => Err(io::Error::new(
                error.kind(),
                format!(
                    "failed to acquire waap state lock {}: {error}",
                    path.display()
                ),
            )),
        }
    }
}

impl Drop for StateLock {
    fn drop(&mut self) {
        if let Err(error) = fs::remove_file(&self.path) {
            if error.kind() != io::ErrorKind::NotFound {
                log::error!(
                    "failed to remove waap state lock {}: {error}",
                    self.path.display()
                );
            }
        }
    }
}

fn validate_state(state_root: &Path, validate: impl Fn(&Path) -> Vec<String>) -> io::Result<()> {
    let errors = validate(state_root);
    if errors.is_empty() {
        Ok(())
    } else {
        Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("waap state is invalid: {}", errors.join("; ")),
        ))
    }
}

fn ensure_path_unstaged(state_root: &Path, path: &Path) -> io::Result<()> {
    let args = vec![
        "diff".into(),
        "--cached".into(),
        "--quiet".into(),
        "--".into(),
        path.as_os_str().to_os_string(),
    ];
    let output = git_output(state_root, &args)?;
    match output.status.code() {
        Some(0) => Ok(()),
        Some(1) => Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!(
                "state mutation path {} has staged changes; commit or unstage it before retrying",
                path.display()
            ),
        )),
        _ => Err(command_error(&args, &output)),
    }
}

fn ensure_state_branch(state_root: &Path) -> io::Result<()> {
    let args = git_args(["branch", "--show-current"]);
    let output = git_output(state_root, &args)?;
    if !output.status.success() {
        return Err(command_error(&args, &output));
    }
    if String::from_utf8_lossy(&output.stdout).trim() != STATE_BRANCH {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!(
                "state worktree {} must check out {STATE_BRANCH}",
                state_root.display()
            ),
        ));
    }
    Ok(())
}

#[cfg(test)]
fn common_git_dir(path: &Path) -> io::Result<PathBuf> {
    let args = git_args(["rev-parse", "--git-common-dir"]);
    let output = git_output(path, &args)?;
    if !output.status.success() {
        return Err(command_error(&args, &output));
    }
    let git_dir = PathBuf::from(String::from_utf8_lossy(&output.stdout).trim());
    let git_dir = if git_dir.is_absolute() {
        git_dir
    } else {
        path.join(git_dir)
    };
    git_dir.canonicalize()
}
