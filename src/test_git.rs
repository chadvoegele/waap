use std::fs;
use std::path::Path;
use std::process::Command;

/// Prevent a Git subprocess from reading the invoking user's configuration.
///
/// Configuration is set on each command instead of through process-global environment variables,
/// so parallel tests cannot interfere with one another.
pub(crate) fn isolate(command: &mut Command) -> &mut Command {
    command
        .env_remove("GIT_CONFIG")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_COUNT", "3")
        .env("GIT_CONFIG_KEY_0", "commit.gpgSign")
        .env("GIT_CONFIG_VALUE_0", "false")
        .env("GIT_CONFIG_KEY_1", "tag.gpgSign")
        .env("GIT_CONFIG_VALUE_1", "false")
        .env("GIT_CONFIG_KEY_2", "core.hooksPath")
        .env("GIT_CONFIG_VALUE_2", "/dev/null")
        .env("GIT_AUTHOR_NAME", "Test")
        .env("GIT_AUTHOR_EMAIL", "test@example.com")
        .env("GIT_COMMITTER_NAME", "Test")
        .env("GIT_COMMITTER_EMAIL", "test@example.com")
}

pub(crate) fn command(root: &Path) -> Command {
    let mut command = Command::new("git");
    command.current_dir(root);
    isolate(&mut command);
    command
}

pub(crate) fn run(root: &Path, args: &[&str]) -> String {
    let output = command(root).args(args).output().unwrap();
    assert!(
        output.status.success(),
        "git {args:?} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).trim().to_string()
}

pub(crate) fn init_repo(root: &Path) {
    run(root, &["init", "-q", "--initial-branch=main"]);
    run(root, &["config", "--local", "user.name", "Test"]);
    run(
        root,
        &["config", "--local", "user.email", "test@example.com"],
    );
}

pub(crate) fn init_repo_with_commit(root: &Path) {
    init_repo(root);
    fs::write(root.join("README.md"), "seed\n").unwrap();
    run(root, &["add", "README.md"]);
    run(root, &["commit", "-q", "-m", "seed"]);
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::os::unix::fs::PermissionsExt;
    use std::process::Command;

    use tempfile::tempdir;

    use super::isolate;

    #[test]
    fn isolation_ignores_conflicting_external_configuration() {
        let dir = tempdir().unwrap();
        let hooks = dir.path().join("hooks");
        fs::create_dir(&hooks).unwrap();
        let hook = hooks.join("pre-commit");
        fs::write(&hook, "#!/bin/sh\nexit 1\n").unwrap();
        fs::set_permissions(&hook, fs::Permissions::from_mode(0o755)).unwrap();

        let hostile_config = dir.path().join("hostile.gitconfig");
        fs::write(
            &hostile_config,
            format!(
                "[init]\n\tdefaultBranch = master\n[commit]\n\tgpgSign = true\n[core]\n\thooksPath = {}\n",
                hooks.display()
            ),
        )
        .unwrap();

        let repo = dir.path().join("repo");
        fs::create_dir(&repo).unwrap();
        let git = |args: &[&str]| {
            let mut command = Command::new("git");
            command
                .current_dir(&repo)
                .env("GIT_CONFIG_GLOBAL", &hostile_config);
            isolate(&mut command);
            let output = command.args(args).output().unwrap();
            assert!(
                output.status.success(),
                "git {args:?} failed: {}",
                String::from_utf8_lossy(&output.stderr)
            );
            String::from_utf8_lossy(&output.stdout).trim().to_string()
        };

        git(&["init", "-q", "--initial-branch=main"]);
        git(&["config", "--local", "user.name", "Test"]);
        git(&["config", "--local", "user.email", "test@example.com"]);
        fs::write(repo.join("README.md"), "seed\n").unwrap();
        git(&["add", "README.md"]);
        git(&["commit", "-q", "-m", "seed"]);

        assert_eq!(git(&["branch", "--show-current"]), "main");
        assert_eq!(
            git(&["log", "-1", "--format=%an <%ae>"]),
            "Test <test@example.com>"
        );
    }
}
