use std::ffi::OsString;
use std::io;
use std::path::Path;
use std::process::{Command, Output};

#[derive(Debug)]
pub(crate) struct Committed<T> {
    pub(crate) value: T,
    pub(crate) commit: String,
}

pub(crate) fn commit_paths(
    repository_root: &Path,
    paths: &[&Path],
    message: &str,
) -> io::Result<String> {
    if paths.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "no paths to commit",
        ));
    }

    let mut add_args: Vec<OsString> = vec!["add".into(), "--".into()];
    add_args.extend(paths.iter().map(|path| path.as_os_str().to_os_string()));
    run(repository_root, &add_args)?;

    let mut diff_args: Vec<OsString> = vec![
        "diff".into(),
        "--cached".into(),
        "--quiet".into(),
        "--".into(),
    ];
    diff_args.extend(paths.iter().map(|path| path.as_os_str().to_os_string()));
    let diff = output(repository_root, &diff_args)?;
    let has_staged_changes = match diff.status.code() {
        Some(0) => false,
        Some(1) => true,
        _ => return Err(command_error(&diff_args, &diff)),
    };

    if has_staged_changes {
        let mut commit_args: Vec<OsString> =
            vec!["commit".into(), "-m".into(), message.into(), "--".into()];
        commit_args.extend(paths.iter().map(|path| path.as_os_str().to_os_string()));
        run(repository_root, &commit_args)?;
    }

    stdout(repository_root, &args(["rev-parse", "HEAD"]))
}

pub(super) fn args<const N: usize>(values: [&str; N]) -> Vec<OsString> {
    values.into_iter().map(OsString::from).collect()
}

pub(super) fn stdout(repository_root: &Path, args: &[OsString]) -> io::Result<String> {
    let output = run(repository_root, args)?;
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_owned())
}

pub(super) fn output(repository_root: &Path, args: &[OsString]) -> io::Result<Output> {
    process(repository_root)
        .args(args)
        .output()
        .map_err(|error| io::Error::new(error.kind(), format!("failed to run git: {error}")))
}

pub(super) fn command_error(args: &[OsString], output: &Output) -> io::Error {
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

pub(super) fn run(repository_root: &Path, args: &[OsString]) -> io::Result<Output> {
    let output = output(repository_root, args)?;
    if !output.status.success() {
        return Err(command_error(args, &output));
    }
    Ok(output)
}

fn process(repository_root: &Path) -> Command {
    let mut command = Command::new("git");
    command.current_dir(repository_root);
    #[cfg(test)]
    crate::test_git::isolate(&mut command);
    command
}
