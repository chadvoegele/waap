use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::json;

use crate::check::{check_state, check_waap};
use crate::cli::OutputFormat;
use crate::git::{
    commit_paths, configure_state_upstream, initialize_state_worktree, query_origin_state_branch,
    OriginStateBranch,
};
use crate::root::ProjectContext;

#[derive(Debug)]
pub(crate) struct RepairReport {
    pub(crate) state_directory: PathBuf,
    pub(crate) migration_commit: Option<String>,
    pub(crate) legacy_removal_commit: Option<String>,
}

/// Repair a state checkout or migrate the invocation worktree's legacy state.
/// An explicit state root is intentionally isolated from legacy-state discovery.
pub(crate) fn repair_project(
    context: &ProjectContext,
    has_explicit_state_root: bool,
) -> io::Result<RepairReport> {
    if has_explicit_state_root {
        validate_central_state(&context.state_root)?;
        configure_state_upstream(&context.primary_repository_root)?;
        return Ok(RepairReport {
            state_directory: context.state_root.clone(),
            migration_commit: None,
            legacy_removal_commit: None,
        });
    }

    let legacy_state = context.invocation_worktree_root.join(".waap");
    let central_exists = context.state_root.exists();
    match (legacy_state.exists(), central_exists) {
        (true, true) => Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            format!(
                "central state {} and legacy state {} coexist; reconcile them manually before retrying waap repair",
                context.state_root.display(),
                legacy_state.display()
            ),
        )),
        (true, false) => migrate_legacy_state(context, &legacy_state),
        (false, true) => {
            validate_central_state(&context.state_root)?;
            configure_state_upstream(&context.primary_repository_root)?;
            Ok(RepairReport {
                state_directory: context.state_root.clone(),
                migration_commit: None,
                legacy_removal_commit: None,
            })
        }
        (false, false) => Err(io::Error::new(
            io::ErrorKind::NotFound,
            "no legacy or central waap state was found; run waap init",
        )),
    }
}

fn migrate_legacy_state(context: &ProjectContext, legacy_state: &Path) -> io::Result<RepairReport> {
    ensure_clean_application_source(&context.invocation_worktree_root)?;
    validate_legacy_state(&context.invocation_worktree_root, legacy_state)?;
    reject_unmigratable_legacy_entries(legacy_state)?;

    // Do not turn a remote state branch plus local legacy state into an
    // implicit merge. Query before creating any central checkout.
    if query_origin_state_branch(&context.primary_repository_root)? == OriginStateBranch::Present {
        return Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            "origin/waap already exists while legacy state is present; reconcile the two states manually",
        ));
    }

    let state_root =
        initialize_state_worktree(&context.primary_repository_root, &context.state_root)?;
    copy_legacy_contents(legacy_state, &state_root)?;
    validate_central_state(&state_root)?;

    let state_paths = [state_root.join("agents"), state_root.join("tickets")];
    let state_path_refs: Vec<&Path> = state_paths.iter().map(PathBuf::as_path).collect();
    let migration_commit = commit_paths(&state_root, &state_path_refs, "waap migrate state")?;

    // The migration commit is durable before any source deletion. If either
    // removal or its application-branch commit fails, both recoverable copies
    // remain and a later invocation reports their coexistence.
    if std::env::var_os("WAAP_REPAIR_FAIL_SOURCE_CLEANUP").is_some() {
        return Err(io::Error::other(
            "injected source cleanup failure after central migration commit",
        ));
    }
    let backup = legacy_backup_path(legacy_state)?;
    fs::rename(legacy_state, &backup)?;
    let legacy_removal_commit = match commit_paths(
        &context.invocation_worktree_root,
        &[legacy_state],
        "Remove legacy waap state",
    ) {
        Ok(commit) => commit,
        Err(error) => {
            return match fs::rename(&backup, legacy_state) {
                Ok(()) => Err(error),
                Err(restore_error) => Err(io::Error::new(
                    error.kind(),
                    format!(
                        "{error}; legacy state was moved to recoverable backup {} but could not be restored to {}: {restore_error}",
                        backup.display(),
                        legacy_state.display()
                    ),
                )),
            };
        }
    };
    fs::remove_dir_all(&backup).map_err(|error| {
        io::Error::new(
            error.kind(),
            format!(
                "legacy state removal was committed, but recoverable backup {} could not be removed: {error}",
                backup.display()
            ),
        )
    })?;

    Ok(RepairReport {
        state_directory: state_root,
        migration_commit: Some(migration_commit),
        legacy_removal_commit: Some(legacy_removal_commit),
    })
}

fn validate_legacy_state(invocation_root: &Path, legacy_state: &Path) -> io::Result<()> {
    if !legacy_state.is_dir() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!(
                "legacy state {} must be a directory",
                legacy_state.display()
            ),
        ));
    }
    let errors = check_waap(invocation_root);
    if errors.is_empty() {
        Ok(())
    } else {
        Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("legacy waap state is invalid: {}", errors.join("; ")),
        ))
    }
}

fn validate_central_state(state_root: &Path) -> io::Result<()> {
    let errors = check_state(state_root);
    if errors.is_empty() {
        Ok(())
    } else {
        Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("central waap state is invalid: {}", errors.join("; ")),
        ))
    }
}

fn reject_unmigratable_legacy_entries(legacy_state: &Path) -> io::Result<()> {
    for entry in fs::read_dir(legacy_state)? {
        let entry = entry?;
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name == "agents" || name == "tickets" || name == ".gitkeep" {
            continue;
        }
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!(
                "legacy state {} contains unsupported top-level entry {name}; move it under agents or tickets before repairing",
                legacy_state.display()
            ),
        ));
    }
    Ok(())
}

fn legacy_backup_path(legacy_state: &Path) -> io::Result<PathBuf> {
    let parent = legacy_state.parent().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("legacy state {} has no parent", legacy_state.display()),
        )
    })?;
    let backup = parent.join(format!(".waap-repair-backup-{}", std::process::id()));
    if backup.exists() {
        return Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            format!(
                "legacy-state recovery backup {} already exists; inspect or remove it before repairing",
                backup.display()
            ),
        ));
    }
    Ok(backup)
}

fn copy_legacy_contents(legacy_state: &Path, state_root: &Path) -> io::Result<()> {
    for name in ["agents", "tickets"] {
        let source = legacy_state.join(name);
        if source.exists() {
            copy_directory(&source, &state_root.join(name))?;
        }
    }
    Ok(())
}

fn copy_directory(source: &Path, destination: &Path) -> io::Result<()> {
    let metadata = fs::symlink_metadata(source)?;
    if metadata.file_type().is_symlink() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("cannot safely migrate symbolic link {}", source.display()),
        ));
    }
    fs::create_dir_all(destination)?;
    for entry in fs::read_dir(source)? {
        let entry = entry?;
        let source = entry.path();
        let destination = destination.join(entry.file_name());
        let metadata = fs::symlink_metadata(&source)?;
        if metadata.file_type().is_symlink() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("cannot safely migrate symbolic link {}", source.display()),
            ));
        }
        if metadata.is_dir() {
            copy_directory(&source, &destination)?;
        } else if metadata.is_file() {
            fs::copy(&source, &destination)?;
        } else {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("cannot safely migrate non-file entry {}", source.display()),
            ));
        }
    }
    Ok(())
}

fn ensure_clean_application_source(root: &Path) -> io::Result<()> {
    let conflicts = git_changed_paths(root, &["diff", "--name-only", "--diff-filter=U"])?;
    if !conflicts.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!(
                "application worktree has unresolved conflicts: {}",
                conflicts.join(", ")
            ),
        ));
    }

    let mut changed = git_changed_paths(root, &["diff", "--name-only"])?;
    changed.extend(git_changed_paths(
        root,
        &["diff", "--cached", "--name-only"],
    )?);
    changed.sort();
    changed.dedup();
    let outside_legacy: Vec<_> = changed
        .into_iter()
        .filter(|path| path != ".waap" && !path.starts_with(".waap/"))
        .collect();
    if outside_legacy.is_empty() {
        Ok(())
    } else {
        Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!(
                "application worktree has staged or unstaged changes outside .waap: {}; commit, stash, or revert them before repairing",
                outside_legacy.join(", ")
            ),
        ))
    }
}

fn git_changed_paths(root: &Path, args: &[&str]) -> io::Result<Vec<String>> {
    let output = Command::new("git").current_dir(root).args(args).output()?;
    if !output.status.success() {
        return Err(io::Error::other(format!(
            "git {} failed: {}",
            args.first().copied().unwrap_or_default(),
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    Ok(String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(str::to_owned)
        .collect())
}

pub(crate) fn print_repair_report(output_format: &OutputFormat, report: &RepairReport) {
    match output_format {
        OutputFormat::Json => println!(
            "{}",
            json!({
                "state_directory": report.state_directory.display().to_string(),
                "migration_commit": report.migration_commit,
                "legacy_removal_commit": report.legacy_removal_commit,
            })
        ),
        OutputFormat::HumanReadable => {
            println!("State directory: {}", report.state_directory.display());
            match (&report.migration_commit, &report.legacy_removal_commit) {
                (Some(migration), Some(removal)) => {
                    println!("Migrated legacy waap state");
                    println!("Migration commit: {migration}");
                    println!("Legacy removal commit: {removal}");
                }
                _ => println!("Waap state is already repaired"),
            }
        }
    }
}
