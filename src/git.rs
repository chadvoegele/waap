#![allow(dead_code)] // Central-state primitives are intentionally unwired until activation.

use std::ffi::OsString;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

pub(crate) fn create_worktree(
    repo_root: &Path,
    branch: &str,
    relative_path: &Path,
) -> io::Result<PathBuf> {
    run_git(
        repo_root,
        &[
            "worktree".into(),
            "add".into(),
            "-b".into(),
            branch.into(),
            relative_path.as_os_str().to_os_string(),
        ],
    )?;
    repo_root.join(relative_path).canonicalize()
}

pub(crate) fn remove_worktree(repo_root: &Path, relative_path: &Path) -> io::Result<()> {
    run_git(
        repo_root,
        &[
            "worktree".into(),
            "remove".into(),
            "--force".into(), // remove worktrees with uncommitted or untracked changes
            relative_path.as_os_str().to_os_string(),
        ],
    )?;
    Ok(())
}

/// The dedicated branch used for central waap state. These primitives remain
/// separate from command dispatch until central-state activation.
pub(crate) const STATE_BRANCH: &str = "waap";
const STATE_BRANCH_REF: &str = "refs/heads/waap";
const STATE_LOCK_FILE: &str = "waap-state.lock";
const ORIGIN_STATE_BRANCH_REF: &str = "refs/remotes/origin/waap";

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct WorktreeRegistration {
    pub(crate) path: PathBuf,
    pub(crate) head: String,
    pub(crate) branch: Option<String>,
}

#[derive(Debug)]
pub(crate) struct StateWorktreeInspection {
    pub(crate) local_branch: Option<String>,
    pub(crate) expected_path_registration: Option<WorktreeRegistration>,
    pub(crate) waap_checkouts: Vec<WorktreeRegistration>,
    pub(crate) upstream_remote: Option<String>,
    pub(crate) upstream_merge: Option<String>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum OriginStateBranch {
    NoOrigin,
    Missing,
    Present,
}

/// Inspect central-state Git metadata without changing refs or worktrees.
pub(crate) fn inspect_state_worktree(
    repository_root: &Path,
    state_root: &Path,
) -> io::Result<StateWorktreeInspection> {
    let registrations = worktree_registrations(repository_root)?;
    let expected_path_registration = registrations
        .iter()
        .find(|registration| paths_match(&registration.path, state_root))
        .cloned();
    let waap_checkouts = registrations
        .into_iter()
        .filter(|registration| registration.branch.as_deref() == Some(STATE_BRANCH_REF))
        .collect();

    Ok(StateWorktreeInspection {
        local_branch: ref_hash(repository_root, STATE_BRANCH_REF)?,
        expected_path_registration,
        waap_checkouts,
        upstream_remote: git_config_value(repository_root, "branch.waap.remote")?,
        upstream_merge: git_config_value(repository_root, "branch.waap.merge")?,
    })
}

/// Determine whether `origin/waap` exists. A failed query is intentionally not
/// treated as a missing branch: callers must not create local state unless the
/// remote result is conclusive.
pub(crate) fn query_origin_state_branch(repository_root: &Path) -> io::Result<OriginStateBranch> {
    if !has_origin(repository_root)? {
        return Ok(OriginStateBranch::NoOrigin);
    }

    let args = os_args([
        "ls-remote",
        "--exit-code",
        "--heads",
        "origin",
        STATE_BRANCH_REF,
    ]);
    let output = git_command(repository_root, &args)?;
    match output.status.code() {
        Some(0) => Ok(OriginStateBranch::Present),
        Some(2) => Ok(OriginStateBranch::Missing),
        _ => Err(run_git_error(&args, &output)),
    }
}

/// Fetch `origin/waap` only after a successful existence query. The fetched
/// ref is then available at `refs/remotes/origin/waap` for validation and
/// adoption.
pub(crate) fn fetch_origin_state_branch(repository_root: &Path) -> io::Result<OriginStateBranch> {
    let state = query_origin_state_branch(repository_root)?;
    if state != OriginStateBranch::Present {
        return Ok(state);
    }

    let refspec = format!("+{STATE_BRANCH_REF}:{ORIGIN_STATE_BRANCH_REF}");
    run_git(
        repository_root,
        &os_args(["fetch", "origin", refspec.as_str()]),
    )?;
    if ref_hash(repository_root, ORIGIN_STATE_BRANCH_REF)?.is_none() {
        return Err(io::Error::other(
            "origin/waap disappeared while it was being fetched; retry initialization",
        ));
    }
    Ok(state)
}

/// Verify that every tree reachable from a single state ref contains only
/// files below `agents/` or `tickets/`. This deliberately never examines any
/// application branch.
pub(crate) fn validate_state_history(repository_root: &Path, revision: &str) -> io::Result<()> {
    let commits = git_stdout(repository_root, &os_args(["rev-list", revision]))?;
    if commits.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("state branch {revision} has no commits"),
        ));
    }

    for commit in commits.lines() {
        let paths = git_stdout(
            repository_root,
            &os_args(["ls-tree", "-r", "--name-only", commit]),
        )?;
        for path in paths.lines() {
            if !is_state_path(path) {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!(
                        "state branch {revision} contains non-state path {path} in commit {commit}; repair or remove the conflicting state history before initializing"
                    ),
                ));
            }
        }
    }
    Ok(())
}

/// Create or adopt the dedicated state worktree. This does not push. It is
/// intentionally not called by normal command dispatch until activation.
pub(crate) fn initialize_state_worktree(
    repository_root: &Path,
    state_root: &Path,
) -> io::Result<PathBuf> {
    let inspection = inspect_state_worktree(repository_root, state_root)?;
    ensure_safe_initialization(repository_root, state_root, &inspection)?;

    match fetch_origin_state_branch(repository_root)? {
        OriginStateBranch::Present => adopt_remote_state_worktree(repository_root, state_root)?,
        OriginStateBranch::NoOrigin | OriginStateBranch::Missing => {
            create_fresh_state_worktree(repository_root, state_root)?
        }
    }

    configure_state_upstream(repository_root)?;
    state_root.canonicalize()
}

/// Return the checked-out state branch commit after initialization or adoption.
pub(crate) fn state_worktree_head(state_root: &Path) -> io::Result<String> {
    git_stdout(state_root, &os_args(["rev-parse", "HEAD"]))
}

fn ensure_safe_initialization(
    repository_root: &Path,
    state_root: &Path,
    inspection: &StateWorktreeInspection,
) -> io::Result<()> {
    if inspection.local_branch.is_some() {
        validate_state_history(repository_root, STATE_BRANCH_REF)?;
        if let Some(registration) = inspection.waap_checkouts.first() {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                format!(
                    "waap is already checked out at {}; use waap repair",
                    registration.path.display()
                ),
            ));
        }
        return Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            "local waap branch exists but has no registered state worktree; use waap repair",
        ));
    }
    if let Some(registration) = &inspection.expected_path_registration {
        return Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            format!(
                "expected state worktree {} is already registered on {}; use waap repair",
                registration.path.display(),
                registration.branch.as_deref().unwrap_or("a detached HEAD")
            ),
        ));
    }
    if let Some(registration) = inspection.waap_checkouts.first() {
        return Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            format!(
                "waap is already checked out at {}; use waap repair",
                registration.path.display()
            ),
        ));
    }
    if state_root.exists() {
        return Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            format!(
                "expected state worktree path {} is occupied and is not registered; choose an empty path or use waap repair",
                state_root.display()
            ),
        ));
    }
    Ok(())
}

fn create_fresh_state_worktree(repository_root: &Path, state_root: &Path) -> io::Result<()> {
    let parent = state_root.parent().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("state worktree path {} has no parent", state_root.display()),
        )
    })?;
    fs::create_dir_all(parent)?;
    let mut worktree_add = os_args(["worktree", "add", "--no-checkout", "--detach"]);
    worktree_add.push(state_root.as_os_str().to_os_string());
    run_git(repository_root, &worktree_add)?;
    run_git(state_root, &os_args(["switch", "--orphan", STATE_BRANCH]))?;

    let agents_marker = state_root.join("agents/.gitkeep");
    let tickets_marker = state_root.join("tickets/.gitkeep");
    fs::create_dir_all(agents_marker.parent().expect("agents marker parent"))?;
    fs::create_dir_all(tickets_marker.parent().expect("tickets marker parent"))?;
    fs::write(&agents_marker, "")?;
    fs::write(&tickets_marker, "")?;
    commit_paths(
        state_root,
        &[agents_marker.as_path(), tickets_marker.as_path()],
        "waap init",
    )?;
    Ok(())
}

fn adopt_remote_state_worktree(repository_root: &Path, state_root: &Path) -> io::Result<()> {
    validate_state_history(repository_root, ORIGIN_STATE_BRANCH_REF)?;
    let parent = state_root.parent().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("state worktree path {} has no parent", state_root.display()),
        )
    })?;
    fs::create_dir_all(parent)?;
    run_git(
        repository_root,
        &os_args(["branch", STATE_BRANCH, ORIGIN_STATE_BRANCH_REF]),
    )?;
    let mut worktree_add = os_args(["worktree", "add"]);
    worktree_add.push(state_root.as_os_str().to_os_string());
    worktree_add.push(STATE_BRANCH.into());
    run_git(repository_root, &worktree_add)?;
    Ok(())
}

fn configure_state_upstream(repository_root: &Path) -> io::Result<()> {
    if has_origin(repository_root)? {
        run_git(
            repository_root,
            &os_args(["config", "branch.waap.remote", "origin"]),
        )?;
        run_git(
            repository_root,
            &os_args(["config", "branch.waap.merge", STATE_BRANCH_REF]),
        )?;
    }
    Ok(())
}

fn worktree_registrations(repository_root: &Path) -> io::Result<Vec<WorktreeRegistration>> {
    let output = git_stdout(
        repository_root,
        &os_args(["worktree", "list", "--porcelain"]),
    )?;
    let mut registrations = Vec::new();
    let mut current: Option<WorktreeRegistration> = None;
    for line in output.lines().chain(std::iter::once("")) {
        if let Some(path) = line.strip_prefix("worktree ") {
            if let Some(registration) = current.take() {
                registrations.push(registration);
            }
            current = Some(WorktreeRegistration {
                path: PathBuf::from(path),
                head: String::new(),
                branch: None,
            });
        } else if let Some(head) = line.strip_prefix("HEAD ") {
            if let Some(registration) = &mut current {
                registration.head = head.to_owned();
            }
        } else if let Some(branch) = line.strip_prefix("branch ") {
            if let Some(registration) = &mut current {
                registration.branch = Some(branch.to_owned());
            }
        } else if line.is_empty() {
            if let Some(registration) = current.take() {
                registrations.push(registration);
            }
        }
    }
    Ok(registrations)
}

fn has_origin(repository_root: &Path) -> io::Result<bool> {
    let remotes = git_stdout(repository_root, &os_args(["remote"]))?;
    Ok(remotes.lines().any(|remote| remote == "origin"))
}

fn ref_hash(repository_root: &Path, reference: &str) -> io::Result<Option<String>> {
    let revision = format!("{reference}^{{commit}}");
    let args = os_args(["rev-parse", "--verify", "--quiet", revision.as_str()]);
    let output = git_command(repository_root, &args)?;
    match output.status.code() {
        Some(0) => Ok(Some(
            String::from_utf8_lossy(&output.stdout).trim().to_owned(),
        )),
        Some(1) => Ok(None),
        _ => Err(run_git_error(&args, &output)),
    }
}

fn git_config_value(repository_root: &Path, key: &str) -> io::Result<Option<String>> {
    let args = os_args(["config", "--get", key]);
    let output = git_command(repository_root, &args)?;
    match output.status.code() {
        Some(0) => Ok(Some(
            String::from_utf8_lossy(&output.stdout).trim().to_owned(),
        )),
        Some(1) => Ok(None),
        _ => Err(run_git_error(&args, &output)),
    }
}

fn git_stdout(repository_root: &Path, args: &[OsString]) -> io::Result<String> {
    let output = run_git(repository_root, args)?;
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_owned())
}

fn os_args<const N: usize>(args: [&str; N]) -> Vec<OsString> {
    args.into_iter().map(OsString::from).collect()
}

fn is_state_path(path: &str) -> bool {
    path.strip_prefix("agents/")
        .is_some_and(|tail| !tail.is_empty())
        || path
            .strip_prefix("tickets/")
            .is_some_and(|tail| !tail.is_empty())
}

fn paths_match(left: &Path, right: &Path) -> bool {
    match (left.canonicalize(), right.canonicalize()) {
        (Ok(left), Ok(right)) => left == right,
        _ => left == right,
    }
}

/// Paths needed to mutate waap state without confusing the state checkout
/// with the application checkout. Legacy dispatch supplies the same checkout
/// for state and source; central-state activation will supply distinct paths.
#[derive(Clone, Debug)]
pub(crate) struct StateMutationContext {
    pub(crate) state_root: PathBuf,
    pub(crate) source_root: PathBuf,
    pub(crate) common_git_dir: PathBuf,
    require_state_branch: bool,
}

impl StateMutationContext {
    pub(crate) fn legacy(waap_root: &Path) -> io::Result<Self> {
        let state_root = waap_root.canonicalize()?;
        let common_git_dir = common_git_dir(&state_root)?;
        Ok(Self {
            source_root: state_root.clone(),
            state_root,
            common_git_dir,
            require_state_branch: false,
        })
    }

    pub(crate) fn central(
        state_root: PathBuf,
        source_root: PathBuf,
        common_git_dir: PathBuf,
    ) -> Self {
        Self {
            state_root,
            source_root,
            common_git_dir,
            require_state_branch: true,
        }
    }

    pub(crate) fn from_project_context(context: &crate::root::ProjectContext) -> Self {
        Self::central(
            context.state_root.clone(),
            context.invocation_worktree_root.clone(),
            context.common_git_dir.clone(),
        )
    }
}

/// A serialized state mutation. It owns the per-repository lock from
/// validation through commit. Call `snapshot_path` before changing each file;
/// dropping an unfinished transaction restores those files and their index
/// entries to HEAD.
pub(crate) struct StateTransaction {
    context: StateMutationContext,
    lock: StateLock,
    snapshots: Vec<PathSnapshot>,
    validate: fn(&Path) -> Vec<String>,
    finished: bool,
}

impl StateTransaction {
    pub(crate) fn begin(
        context: StateMutationContext,
        validate: fn(&Path) -> Vec<String>,
    ) -> io::Result<Self> {
        let lock = StateLock::acquire(&context.common_git_dir)?;
        if context.require_state_branch {
            ensure_state_branch(&context.state_root)?;
        }
        validate_state(&context.state_root, validate)?;
        Ok(Self {
            context,
            lock,
            snapshots: Vec::new(),
            validate,
            finished: false,
        })
    }

    pub(crate) fn state_root(&self) -> &Path {
        &self.context.state_root
    }

    pub(crate) fn source_root(&self) -> &Path {
        &self.context.source_root
    }

    pub(crate) fn snapshot_path(&mut self, path: &Path) -> io::Result<()> {
        if !path.starts_with(&self.context.state_root) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!(
                    "state mutation path {} is outside state directory {}",
                    path.display(),
                    self.context.state_root.display()
                ),
            ));
        }
        if self.snapshots.iter().any(|snapshot| snapshot.path == path) {
            return Ok(());
        }
        ensure_path_unstaged(&self.context.state_root, path)?;
        self.snapshots.push(PathSnapshot {
            path: path.to_path_buf(),
            contents: if path.exists() {
                Some(fs::read(path)?)
            } else {
                None
            },
        });
        Ok(())
    }

    pub(crate) fn validate(&self, validate: impl Fn(&Path) -> Vec<String>) -> io::Result<()> {
        validate_state(&self.context.state_root, validate)
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
        match commit_paths(&self.context.state_root, paths, message) {
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
            run_git(&self.context.state_root, &reset_args)?;
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
        Ok(())
    }
}

impl Drop for StateTransaction {
    fn drop(&mut self) {
        if !self.finished {
            if let Err(error) = self.restore() {
                log::error!("failed to roll back waap state transaction: {error}");
            }
        }
        // Keep the lock field alive until after rollback.
        let _ = &self.lock;
    }
}

struct PathSnapshot {
    path: PathBuf,
    contents: Option<Vec<u8>>,
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
                    "waap state transaction is already running ({}) ; retry after it finishes",
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
    let output = git_command(state_root, &args)?;
    match output.status.code() {
        Some(0) => Ok(()),
        Some(1) => Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!(
                "state mutation path {} has staged changes; commit or unstage it before retrying",
                path.display()
            ),
        )),
        _ => Err(run_git_error(&args, &output)),
    }
}

fn ensure_state_branch(state_root: &Path) -> io::Result<()> {
    let output = git_command(state_root, &os_args(["branch", "--show-current"]))?;
    if !output.status.success() {
        return Err(run_git_error(
            &os_args(["branch", "--show-current"]),
            &output,
        ));
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

fn common_git_dir(path: &Path) -> io::Result<PathBuf> {
    let output = git_command(path, &os_args(["rev-parse", "--git-common-dir"]))?;
    if !output.status.success() {
        return Err(run_git_error(
            &os_args(["rev-parse", "--git-common-dir"]),
            &output,
        ));
    }
    let git_dir = PathBuf::from(String::from_utf8_lossy(&output.stdout).trim());
    let git_dir = if git_dir.is_absolute() {
        git_dir
    } else {
        path.join(git_dir)
    };
    git_dir.canonicalize()
}

#[derive(Debug)]
pub(crate) struct Committed<T> {
    pub(crate) value: T,
    pub(crate) commit: String,
}

pub(crate) fn commit_paths(waap_root: &Path, paths: &[&Path], message: &str) -> io::Result<String> {
    if paths.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "no paths to commit",
        ));
    }

    let mut add_args: Vec<OsString> = vec!["add".into(), "--".into()];
    add_args.extend(paths.iter().map(|path| path.as_os_str().to_os_string()));
    run_git(waap_root, &add_args)?;

    let mut diff_args: Vec<OsString> = vec![
        "diff".into(),
        "--cached".into(),
        "--quiet".into(),
        "--".into(),
    ];
    diff_args.extend(paths.iter().map(|path| path.as_os_str().to_os_string()));
    let diff = git_command(waap_root, &diff_args)?;
    let has_staged_changes = match diff.status.code() {
        Some(0) => false,
        Some(1) => true,
        _ => return Err(run_git_error(&diff_args, &diff)),
    };

    if has_staged_changes {
        let mut commit_args: Vec<OsString> =
            vec!["commit".into(), "-m".into(), message.into(), "--".into()];
        commit_args.extend(paths.iter().map(|path| path.as_os_str().to_os_string()));
        run_git(waap_root, &commit_args)?;
    }

    let output = run_git(waap_root, &["rev-parse".into(), "HEAD".into()])?;
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

pub(crate) fn is_inside_git_work_tree(path: &Path) -> io::Result<bool> {
    let output = git_process(path)
        .args(["rev-parse", "--is-inside-work-tree"])
        .output()
        .map_err(|error| io::Error::new(error.kind(), format!("failed to run git: {error}")))?;
    Ok(output.status.success() && String::from_utf8_lossy(&output.stdout).trim() == "true")
}

fn git_process(waap_root: &Path) -> Command {
    let mut command = Command::new("git");
    command.current_dir(waap_root);
    #[cfg(test)]
    crate::test_git::isolate(&mut command);
    command
}

fn git_command(waap_root: &Path, args: &[OsString]) -> io::Result<Output> {
    git_process(waap_root)
        .args(args)
        .output()
        .map_err(|error| io::Error::new(error.kind(), format!("failed to run git: {error}")))
}

fn run_git_error(args: &[OsString], output: &Output) -> io::Error {
    let stderr = String::from_utf8_lossy(&output.stderr);
    let stderr = stderr.trim();
    let subcommand = args
        .first()
        .map(|arg| arg.to_string_lossy().into_owned())
        .unwrap_or_default();
    let detail = if stderr.is_empty() {
        format!("git {subcommand} exited with {}", output.status)
    } else {
        format!("git {subcommand} failed: {stderr}")
    };
    io::Error::other(detail)
}

fn run_git(waap_root: &Path, args: &[OsString]) -> io::Result<Output> {
    let output = git_command(waap_root, args)?;

    if !output.status.success() {
        return Err(run_git_error(args, &output));
    }

    Ok(output)
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::io;
    use std::path::{Path, PathBuf};
    use std::process::Command;

    use tempfile::tempdir;

    use super::{
        commit_paths, create_worktree, fetch_origin_state_branch, initialize_state_worktree,
        inspect_state_worktree, is_inside_git_work_tree, query_origin_state_branch,
        remove_worktree, OriginStateBranch, StateMutationContext, StateTransaction, STATE_BRANCH,
    };
    use crate::test_git::{init_repo, init_repo_with_commit, isolate, run};

    fn write_file(path: &Path, contents: &str) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, contents).unwrap();
    }

    fn central_context(root: &Path) -> StateMutationContext {
        StateMutationContext::central(
            root.canonicalize().unwrap(),
            root.canonicalize().unwrap(),
            root.join(".git").canonicalize().unwrap(),
        )
    }

    fn central_state_is_valid(root: &Path) -> Vec<String> {
        if root.join("agents").is_dir() && root.join("tickets").is_dir() {
            Vec::new()
        } else {
            vec!["missing central state directories".to_string()]
        }
    }

    fn central_state_marker_is_valid(root: &Path) -> Vec<String> {
        if central_state_is_valid(root).is_empty()
            && fs::read_to_string(root.join("agents/.gitkeep"))
                .is_ok_and(|contents| contents.is_empty())
        {
            Vec::new()
        } else {
            vec!["central state marker is invalid".to_string()]
        }
    }

    fn init_central_state_repo(root: &Path) {
        init_repo(root);
        run(root, &["switch", "--orphan", STATE_BRANCH]);
        write_file(&root.join("agents/.gitkeep"), "");
        write_file(&root.join("tickets/.gitkeep"), "");
        run(root, &["add", "agents", "tickets"]);
        run(root, &["commit", "-q", "-m", "state seed"]);
    }

    #[test]
    fn state_transaction_commits_only_explicit_central_paths_and_returns_hash() {
        let dir = tempdir().unwrap();
        init_central_state_repo(dir.path());
        let changed = dir.path().join("agents/aa-one/agent.md");
        let unrelated = dir.path().join("tickets/tt-unrelated/ticket.md");
        write_file(&unrelated, "user staged state\n");
        run(dir.path(), &["add", "tickets"]);

        let mut transaction =
            StateTransaction::begin(central_context(dir.path()), central_state_is_valid).unwrap();
        transaction.snapshot_path(&changed).unwrap();
        write_file(&changed, "agent state\n");
        let commit = transaction
            .commit(&[changed.as_path()], "waap agent update aa-one")
            .unwrap();

        assert_eq!(commit, run(dir.path(), &["rev-parse", "HEAD"]));
        assert_eq!(
            run(
                dir.path(),
                &["show", "--name-only", "--pretty=format:", "HEAD"]
            ),
            "agents/aa-one/agent.md"
        );
        assert!(run(dir.path(), &["diff", "--cached", "--name-only"])
            .contains("tickets/tt-unrelated/ticket.md"));
    }

    #[test]
    fn state_transaction_noop_returns_head_without_a_commit() {
        let dir = tempdir().unwrap();
        init_central_state_repo(dir.path());
        let path = dir.path().join("agents/.gitkeep");
        let before = run(dir.path(), &["rev-parse", "HEAD"]);

        let mut transaction =
            StateTransaction::begin(central_context(dir.path()), central_state_is_valid).unwrap();
        transaction.snapshot_path(&path).unwrap();
        write_file(&path, "");
        let commit = transaction.commit(&[path.as_path()], "no-op").unwrap();

        assert_eq!(commit, before);
        assert_eq!(run(dir.path(), &["rev-parse", "HEAD"]), before);
    }

    #[test]
    fn state_transaction_rolls_back_invalid_or_failed_commits() {
        let dir = tempdir().unwrap();
        init_central_state_repo(dir.path());
        let invalid = dir.path().join("agents/.gitkeep");
        let mut transaction =
            StateTransaction::begin(central_context(dir.path()), central_state_marker_is_valid)
                .unwrap();
        transaction.snapshot_path(&invalid).unwrap();
        write_file(&invalid, "changed\n");
        let error = transaction
            .commit(&[invalid.as_path()], "must fail")
            .unwrap_err();

        assert!(error
            .to_string()
            .contains("central state marker is invalid"));
        assert_eq!(fs::read_to_string(&invalid).unwrap(), "");
        assert!(run(dir.path(), &["diff", "--cached", "--name-only"]).is_empty());
        assert!(run(dir.path(), &["status", "--porcelain"]).is_empty());
    }

    #[test]
    fn state_transaction_lock_contention_fails_without_changing_state() {
        let dir = tempdir().unwrap();
        init_central_state_repo(dir.path());
        let transaction =
            StateTransaction::begin(central_context(dir.path()), central_state_is_valid).unwrap();
        let error =
            match StateTransaction::begin(central_context(dir.path()), central_state_is_valid) {
                Ok(_) => panic!("competing transaction unexpectedly acquired the lock"),
                Err(error) => error,
            };

        assert_eq!(error.kind(), io::ErrorKind::WouldBlock);
        assert!(error.to_string().contains("retry"));
        drop(transaction);
        assert!(
            StateTransaction::begin(central_context(dir.path()), central_state_is_valid).is_ok()
        );
        assert!(run(dir.path(), &["status", "--porcelain"]).is_empty());
    }

    #[test]
    fn commit_paths_creates_single_commit_with_returned_hash() {
        let dir = tempdir().unwrap();
        init_repo(dir.path());
        let file = dir.path().join(".waap/tickets/tt-x/ticket.md");
        write_file(&file, "+++\n+++\n");

        let count_before = run(dir.path(), &["rev-list", "--count", "--all"])
            .parse::<u32>()
            .unwrap_or(0);
        let hash = commit_paths(dir.path(), &[file.as_path()], "waap ticket new tt-x").unwrap();
        let count_after: u32 = run(dir.path(), &["rev-list", "--count", "HEAD"])
            .parse()
            .unwrap();

        assert_eq!(count_after, count_before + 1);
        assert_eq!(run(dir.path(), &["rev-parse", "HEAD"]), hash);
        assert_eq!(
            run(dir.path(), &["log", "-1", "--pretty=%s"]),
            "waap ticket new tt-x"
        );
    }

    #[test]
    fn commit_paths_only_stages_given_files() {
        let dir = tempdir().unwrap();
        init_repo(dir.path());
        // An unrelated file that is already staged must not be swept into the commit.
        let unrelated = dir.path().join("unrelated.txt");
        write_file(&unrelated, "user change\n");
        run(dir.path(), &["add", "unrelated.txt"]);

        let tracked = dir.path().join(".waap/agents/aa-00000001/agent.md");
        write_file(&tracked, "+++\n+++\n");

        commit_paths(
            dir.path(),
            &[tracked.as_path()],
            "waap agent new aa-00000001",
        )
        .unwrap();

        let committed = run(
            dir.path(),
            &["show", "--name-only", "--pretty=format:", "HEAD"],
        );
        assert!(committed.contains(".waap/agents/aa-00000001/agent.md"));
        assert!(!committed.contains("unrelated.txt"));
        // The unrelated change is still staged and uncommitted.
        let staged = run(dir.path(), &["diff", "--cached", "--name-only"]);
        assert!(staged.contains("unrelated.txt"));
    }

    #[test]
    fn commit_paths_commits_only_changed_path_among_many_modifications() {
        let dir = tempdir().unwrap();
        init_repo(dir.path());
        let a = dir.path().join(".waap/agents/aa-00000001/agent.md");
        let b = dir.path().join(".waap/agents/aa-00000002/agent.md");
        write_file(&a, "+++\nstatus = \"ready\"\n+++\n");
        write_file(&b, "+++\nstatus = \"ready\"\n+++\n");
        run(dir.path(), &["add", "-A"]);
        run(dir.path(), &["commit", "-q", "-m", "seed"]);

        // Modify both, but only commit one.
        write_file(&a, "+++\nstatus = \"running\"\n+++\n");
        write_file(&b, "+++\nstatus = \"running\"\n+++\n");

        commit_paths(dir.path(), &[a.as_path()], "waap agent run aa-00000001").unwrap();

        let committed = run(
            dir.path(),
            &["show", "--name-only", "--pretty=format:", "HEAD"],
        );
        assert!(committed.contains("aa-00000001"));
        assert!(!committed.contains("aa-00000002"));
    }

    #[test]
    fn commit_paths_noop_returns_head_without_new_commit() {
        let dir = tempdir().unwrap();
        init_repo(dir.path());
        let file = dir.path().join(".waap/agents/aa-00000001/agent.md");
        write_file(&file, "+++\nstatus = \"completed\"\n+++\n");
        // First write creates the commit; the second writes identical contents (no staged diff).
        let first =
            commit_paths(dir.path(), &[file.as_path()], "waap agent run aa-00000001").unwrap();

        let count_before = run(dir.path(), &["rev-list", "--count", "HEAD"])
            .parse::<u32>()
            .unwrap();
        let second =
            commit_paths(dir.path(), &[file.as_path()], "waap agent run aa-00000001").unwrap();
        let count_after = run(dir.path(), &["rev-list", "--count", "HEAD"])
            .parse::<u32>()
            .unwrap();

        assert_eq!(count_after, count_before, "no new commit should be created");
        assert_eq!(second, first, "the current HEAD is returned for a no-op");
        assert_eq!(run(dir.path(), &["rev-parse", "HEAD"]), second);
    }

    #[test]
    fn commit_paths_reports_failure_when_not_a_git_repo() {
        let dir = tempdir().unwrap();
        let file = dir.path().join(".waap/tickets/tt-x/ticket.md");
        write_file(&file, "+++\n+++\n");

        let error =
            commit_paths(dir.path(), &[file.as_path()], "waap ticket new tt-x").unwrap_err();

        assert!(!error.to_string().is_empty());
    }

    #[test]
    fn commit_paths_respects_waap_root() {
        let dir = tempdir().unwrap();
        let waap_root = dir.path().join("nested/repo");
        fs::create_dir_all(&waap_root).unwrap();
        init_repo(&waap_root);
        let file = waap_root.join(".waap/tickets/tt-x/ticket.md");
        write_file(&file, "+++\n+++\n");

        let hash = commit_paths(&waap_root, &[file.as_path()], "waap ticket new tt-x").unwrap();

        assert_eq!(run(&waap_root, &["rev-parse", "HEAD"]), hash);
        assert_eq!(
            run(&waap_root, &["log", "-1", "--pretty=%s"]),
            "waap ticket new tt-x"
        );
    }

    #[test]
    fn commit_paths_rejects_empty_paths() {
        let dir = tempdir().unwrap();
        init_repo(dir.path());

        let error = commit_paths(dir.path(), &[], "waap ticket new tt-x").unwrap_err();

        assert_eq!(error.kind(), std::io::ErrorKind::InvalidInput);
    }

    #[test]
    fn is_inside_git_work_tree_true_for_git_repo() {
        let dir = tempdir().unwrap();
        init_repo(dir.path());

        assert!(is_inside_git_work_tree(dir.path()).unwrap());
    }

    #[test]
    fn is_inside_git_work_tree_false_outside_git_repo() {
        let dir = tempdir().unwrap();

        assert!(!is_inside_git_work_tree(dir.path()).unwrap());
    }

    #[test]
    fn create_worktree_creates_checkout_at_requested_path_and_branch() {
        let dir = tempdir().unwrap();
        init_repo_with_commit(dir.path());
        let relative_path = Path::new("checkouts/topic");

        let worktree = create_worktree(dir.path(), "topic-branch", relative_path).unwrap();

        assert!(worktree.is_dir());
        assert_eq!(
            worktree,
            dir.path().join(relative_path).canonicalize().unwrap()
        );
        // The seed commit's tree is checked out in the worktree.
        assert!(worktree.join("README.md").exists());
        let branches = run(dir.path(), &["branch", "--list", "topic-branch"]);
        assert!(branches.contains("topic-branch"));
        let worktrees = run(dir.path(), &["worktree", "list"]);
        assert!(worktrees.contains("checkouts/topic"));
    }

    #[test]
    fn remove_worktree_deletes_requested_checkout() {
        let dir = tempdir().unwrap();
        init_repo_with_commit(dir.path());
        let relative_path = Path::new("checkouts/topic");
        let worktree = create_worktree(dir.path(), "topic-branch", relative_path).unwrap();

        remove_worktree(dir.path(), relative_path).unwrap();

        assert!(!worktree.exists());
        let worktrees = run(dir.path(), &["worktree", "list"]);
        assert!(!worktrees.contains("checkouts/topic"));
    }

    #[test]
    fn remove_worktree_forces_removal_with_uncommitted_changes() {
        let dir = tempdir().unwrap();
        init_repo_with_commit(dir.path());
        let relative_path = Path::new("checkouts/topic");
        let worktree = create_worktree(dir.path(), "topic-branch", relative_path).unwrap();
        // Leave dirty state behind, as an agent that exits early or fails would.
        write_file(&worktree.join("scratch.txt"), "uncommitted work\n");

        remove_worktree(dir.path(), relative_path).unwrap();

        assert!(!worktree.exists());
    }

    #[test]
    fn fresh_state_worktree_is_orphaned_tracks_origin_and_preserves_application_checkout() {
        let dir = tempdir().unwrap();
        init_repo_with_commit(dir.path());
        let central = tempdir().unwrap();
        let remote = bare_remote(central.path());
        add_origin(dir.path(), &remote);
        let state = central.path().join("state");
        let application_head = run(dir.path(), &["rev-parse", "HEAD"]);

        let created = initialize_state_worktree(dir.path(), &state).unwrap();

        assert_eq!(created, state.canonicalize().unwrap());
        assert_eq!(run(dir.path(), &["rev-parse", "HEAD"]), application_head);
        assert!(run(dir.path(), &["status", "--porcelain"]).is_empty());
        assert!(state.join("agents").is_dir());
        assert!(state.join("tickets").is_dir());
        assert_eq!(
            run(dir.path(), &["rev-list", "--parents", "-1", STATE_BRANCH])
                .split_whitespace()
                .count(),
            1
        );
        assert_eq!(
            run(dir.path(), &["ls-tree", "-r", "--name-only", STATE_BRANCH]),
            "agents/.gitkeep\ntickets/.gitkeep"
        );

        let inspection = inspect_state_worktree(dir.path(), &state).unwrap();
        assert!(inspection.local_branch.is_some());
        assert_eq!(
            inspection
                .expected_path_registration
                .unwrap()
                .branch
                .as_deref(),
            Some("refs/heads/waap")
        );
        assert_eq!(inspection.upstream_remote.as_deref(), Some("origin"));
        assert_eq!(
            inspection.upstream_merge.as_deref(),
            Some("refs/heads/waap")
        );
    }

    #[test]
    fn adopts_only_verified_remote_state_without_inspecting_application_history() {
        let dir = tempdir().unwrap();
        init_repo_with_commit(dir.path());
        let central = tempdir().unwrap();
        let remote = bare_remote(central.path());
        let source_dir = tempdir().unwrap();
        let source = source_dir.path().join("remote-source");
        fs::create_dir(&source).unwrap();
        init_repo_with_commit(&source);
        run(&source, &["switch", "--orphan", STATE_BRANCH]);
        write_file(&source.join("agents/aa-one/agent.md"), "+++");
        write_file(&source.join("tickets/tt-one/ticket.md"), "+++");
        run(&source, &["add", "agents", "tickets"]);
        run(&source, &["commit", "-q", "-m", "remote state"]);
        run(
            &source,
            &["remote", "add", "origin", remote.to_str().unwrap()],
        );
        run(&source, &["push", "-q", "origin", STATE_BRANCH]);
        let remote_head = run(&source, &["rev-parse", STATE_BRANCH]);
        add_origin(dir.path(), &remote);
        let state = central.path().join("state");
        let application_head = run(dir.path(), &["rev-parse", "HEAD"]);

        initialize_state_worktree(dir.path(), &state).unwrap();

        assert_eq!(run(&state, &["rev-parse", "HEAD"]), remote_head);
        assert!(state.join("agents/aa-one/agent.md").is_file());
        assert!(state.join("tickets/tt-one/ticket.md").is_file());
        assert_eq!(run(dir.path(), &["rev-parse", "HEAD"]), application_head);
        assert!(run(dir.path(), &["status", "--porcelain"]).is_empty());
    }

    #[test]
    fn rejects_remote_waap_history_with_application_paths() {
        let dir = tempdir().unwrap();
        init_repo_with_commit(dir.path());
        let central = tempdir().unwrap();
        let remote = bare_remote(central.path());
        let source_dir = tempdir().unwrap();
        init_repo_with_commit(source_dir.path());
        run(source_dir.path(), &["branch", STATE_BRANCH, "main"]);
        run(
            source_dir.path(),
            &["remote", "add", "origin", remote.to_str().unwrap()],
        );
        run(source_dir.path(), &["push", "-q", "origin", STATE_BRANCH]);
        add_origin(dir.path(), &remote);
        let state = central.path().join("state");

        let error = initialize_state_worktree(dir.path(), &state).unwrap_err();

        assert!(error.to_string().contains("non-state path README.md"));
        assert!(!state.exists());
        assert!(run(dir.path(), &["branch", "--list", STATE_BRANCH]).is_empty());
    }

    #[test]
    fn missing_remote_state_creates_fresh_but_unreachable_origin_does_not() {
        let dir = tempdir().unwrap();
        init_repo_with_commit(dir.path());
        let remote = bare_remote(dir.path());
        add_origin(dir.path(), &remote);
        let state = dir.path().join("central/state");

        assert_eq!(
            query_origin_state_branch(dir.path()).unwrap(),
            OriginStateBranch::Missing
        );
        assert_eq!(
            fetch_origin_state_branch(dir.path()).unwrap(),
            OriginStateBranch::Missing
        );
        initialize_state_worktree(dir.path(), &state).unwrap();
        assert!(state.is_dir());

        let unreachable = tempdir().unwrap();
        init_repo_with_commit(unreachable.path());
        run(
            unreachable.path(),
            &[
                "remote",
                "add",
                "origin",
                unreachable.path().join("missing-remote").to_str().unwrap(),
            ],
        );
        let missing_state = unreachable.path().join("central/state");
        let error = initialize_state_worktree(unreachable.path(), &missing_state).unwrap_err();

        assert!(error.to_string().contains("ls-remote"));
        assert!(!missing_state.exists());
        assert!(run(unreachable.path(), &["branch", "--list", STATE_BRANCH]).is_empty());
    }

    #[test]
    fn rejects_non_state_history_and_existing_state_conflicts_without_resetting_them() {
        let dir = tempdir().unwrap();
        init_repo_with_commit(dir.path());
        run(dir.path(), &["branch", STATE_BRANCH]);
        let state = dir.path().join("central/state");

        let error = initialize_state_worktree(dir.path(), &state).unwrap_err();
        assert!(error.to_string().contains("non-state path README.md"));
        assert!(
            run(dir.path(), &["show-ref", "--verify", "refs/heads/waap"])
                .contains("refs/heads/waap")
        );
        assert!(!state.exists());

        let occupied = tempdir().unwrap();
        init_repo_with_commit(occupied.path());
        let occupied_state = occupied.path().join("central/state");
        fs::create_dir_all(&occupied_state).unwrap();
        let error = initialize_state_worktree(occupied.path(), &occupied_state).unwrap_err();
        assert!(error.to_string().contains("is occupied"));
        assert!(run(occupied.path(), &["branch", "--list", STATE_BRANCH]).is_empty());
    }

    #[test]
    fn detects_waap_checked_out_outside_the_expected_state_path() {
        let dir = tempdir().unwrap();
        init_repo_with_commit(dir.path());
        run(dir.path(), &["switch", "--orphan", STATE_BRANCH]);
        write_file(&dir.path().join("agents/.gitkeep"), "");
        write_file(&dir.path().join("tickets/.gitkeep"), "");
        run(dir.path(), &["add", "agents", "tickets"]);
        run(dir.path(), &["commit", "-q", "-m", "state"]);
        let expected_state = dir.path().join("central/state");

        let inspection = inspect_state_worktree(dir.path(), &expected_state).unwrap();
        assert_eq!(inspection.waap_checkouts.len(), 1);
        assert_eq!(inspection.waap_checkouts[0].path, dir.path());
        let error = initialize_state_worktree(dir.path(), &expected_state).unwrap_err();

        assert!(error.to_string().contains("already checked out"));
        assert!(!expected_state.exists());
    }

    fn bare_remote(root: &Path) -> PathBuf {
        let remote = root.join("remote.git");
        let mut command = Command::new("git");
        isolate(&mut command);
        let output = command
            .args(["init", "-q", "--bare", remote.to_str().unwrap()])
            .output()
            .unwrap();
        assert!(output.status.success());
        remote
    }

    fn add_origin(repository: &Path, remote: &Path) {
        run(
            repository,
            &["remote", "add", "origin", remote.to_str().unwrap()],
        );
    }
}
