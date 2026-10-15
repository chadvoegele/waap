//! Exercise the real CLI lifecycle with a local protocol double, without an external agent.
#![cfg(target_os = "linux")]
mod common;

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::process::{Command, Output, Stdio};

use common::{git, init_repo, isolate_git_config};

fn waap(root: &Path, args: &[&str], mode: &str) -> Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_waap"));
    isolate_git_config(&mut command);
    command
        .current_dir(root)
        .env("GIT_CONFIG_VALUE_2", root.join(".git/hooks"))
        .env(
            "PATH",
            format!(
                "{}:{}",
                root.join("bin").display(),
                std::env::var("PATH").unwrap()
            ),
        )
        .env("WAAP_TEST_MODE", mode)
        .env("WAAP_TEST_PID_FILE", root.join("backend.pid"))
        .env_remove("CODEX_MODEL")
        .env_remove("CODEX_REASONING_EFFORT")
        .args(["--waap-root", root.join(".state").to_str().unwrap()])
        .args(args)
        .stdin(Stdio::null())
        .output()
        .unwrap()
}

fn seed_backend(root: &Path) {
    fs::create_dir(root.join("bin")).unwrap();
    let path = root.join("bin/codex");
    fs::write(
        &path,
        r#"#!/usr/bin/python3
import json, os, sys, time
with open(os.environ['WAAP_TEST_PID_FILE'], 'w') as file:
    file.write(str(os.getpid()))
mode = os.environ['WAAP_TEST_MODE']
def emit(value):
    print(json.dumps(value), flush=True)
for line in sys.stdin:
    request = json.loads(line)
    method = request['method']
    if 'id' not in request:
        continue
    if mode == 'malformed':
        print('malformed JSON', flush=True)
        time.sleep(60)
    result = {}
    if method == 'thread/start':
        result = {'thread': {'id': 'th_test'}}
    if method == 'turn/start':
        result = {'turn': {'id': 'tu_test'}}
    emit({'id': request['id'], 'result': result})
    if method == 'thread/start' and mode == 'publication-failed':
        time.sleep(60)
    if method == 'turn/start':
        emit({'method': 'turn/completed', 'params': {
            'threadId': 'th_test', 'turn': {'id': 'tu_test',
            'status': 'failed' if mode == 'failed' else 'completed'}}})
        time.sleep(60)
"#,
    )
    .unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
}

#[test]
fn failed_initial_claim_restores_ready_record_and_allows_retry() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    init_repo(root);
    seed_backend(root);
    assert!(waap(root, &["init"], "completed").status.success());
    assert!(waap(root, &["agent", "new", "--name", "Test"], "completed")
        .status
        .success());
    let path = root.join(".state/agents/aa-test/agent.md");
    let original = fs::read_to_string(&path).unwrap();
    let head = git(&root.join(".state"), &["rev-parse", "HEAD"]);
    let hook = root.join(".git/hooks/pre-commit");
    fs::write(&hook, "#!/bin/sh\nrm -- \"$0\"\nexit 1\n").unwrap();
    fs::set_permissions(&hook, fs::Permissions::from_mode(0o755)).unwrap();
    let args = ["agent", "run", "--agent-id", "aa-test", "--system", "codex"];
    let failed = waap(root, &args, "completed");
    assert!(!failed.status.success());
    assert!(String::from_utf8_lossy(&failed.stderr).contains("git commit"));
    assert_eq!(fs::read_to_string(&path).unwrap(), original);
    assert_eq!(git(&root.join(".state"), &["rev-parse", "HEAD"]), head);
    assert!(!root.join("backend.pid").exists());
    assert!(!root.join("worktrees/aa-test").exists());
    assert!(waap(root, &["check"], "completed").status.success());
    let retry = waap(root, &args, "completed");
    assert!(retry.status.success(), "{retry:?}");
    assert!(fs::read_to_string(path)
        .unwrap()
        .contains("status = \"completed\""));
}

#[test]
fn codex_success_failure_and_malformed_startup_reap_process_and_clean_worktree() {
    for (mode, status, success) in [
        ("completed", "completed", true),
        ("failed", "failed", false),
        ("malformed", "failed", false),
        ("publication-failed", "failed", false),
    ] {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        init_repo(root);
        seed_backend(root);
        assert!(waap(root, &["init"], mode).status.success());
        assert!(waap(root, &["agent", "new", "--name", "Test"], mode)
            .status
            .success());
        if mode == "publication-failed" {
            let path = root.join(".state/agents/aa-test/agent.md");
            let contents = fs::read_to_string(&path).unwrap().replace(
                "status = \"ready\"",
                "status = \"ready\"\nsession_id = \"existing\"",
            );
            fs::write(&path, contents).unwrap();
            git(&root.join(".state"), &["add", "agents/aa-test/agent.md"]);
            git(
                &root.join(".state"),
                &["commit", "-q", "-m", "Seed conflicting session"],
            );
        }
        let output = waap(
            root,
            &["agent", "run", "--agent-id", "aa-test", "--system", "codex"],
            mode,
        );
        assert_eq!(
            output.status.success(),
            success,
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let record = fs::read_to_string(root.join(".state/agents/aa-test/agent.md")).unwrap();
        assert!(record.contains(&format!("status = \"{status}\"")));
        if mode == "publication-failed" {
            assert!(record.contains("session_id = \"existing\""));
            assert!(String::from_utf8_lossy(&output.stderr).contains("already has session id"));
        } else if mode != "malformed" {
            assert!(record.contains("session_id = \"th_test\""));
        } else {
            assert!(String::from_utf8_lossy(&output.stderr).contains("malformed JSON"));
        }
        let pid = fs::read_to_string(root.join("backend.pid")).unwrap();
        assert!(
            !Path::new(&format!("/proc/{pid}")).exists(),
            "backend was not reaped"
        );
        assert!(!root.join("worktrees/aa-test").exists());
        assert!(!git(root, &["worktree", "list"]).contains("worktrees/aa-test"));
        assert!(waap(root, &["check"], mode).status.success());
    }
}
