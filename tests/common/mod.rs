use std::path::Path;
use std::process::Command;

pub fn isolate_git_config(command: &mut Command) -> &mut Command {
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

pub fn git(waap_root: &Path, args: &[&str]) -> String {
    let mut command = Command::new("git");
    command.current_dir(waap_root);
    isolate_git_config(&mut command);
    let output = command.args(args).output().unwrap();
    assert!(
        output.status.success(),
        "git {args:?} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).trim().to_string()
}

pub fn init_repo(waap_root: &Path) {
    git(waap_root, &["init", "-q", "--initial-branch=main"]);
    git(waap_root, &["config", "--local", "user.name", "Test"]);
    git(
        waap_root,
        &["config", "--local", "user.email", "test@example.com"],
    );
    std::fs::write(waap_root.join("README.md"), "seed\n").unwrap();
    git(waap_root, &["add", "README.md"]);
    git(waap_root, &["commit", "-q", "-m", "seed"]);
}
