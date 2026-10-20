//! Exercise the real CLI lifecycle with a local protocol double, without an external agent.
#![cfg(target_os = "linux")]
mod common;

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::process::{Command, Output, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use serde_json::{json, Value};

use common::{git, init_repo, isolate_git_config};

fn waap(root: &Path, args: &[&str], mode: &str) -> Output {
    let runtime = tempfile::tempdir().unwrap();
    fs::set_permissions(runtime.path(), fs::Permissions::from_mode(0o700)).unwrap();
    waap_command(root, mode, runtime.path())
        .args(args)
        .output()
        .unwrap()
}

fn waap_command(root: &Path, mode: &str, runtime: &Path) -> Command {
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
        .env("XDG_RUNTIME_DIR", runtime)
        .env("WAAP_TEST_PID_FILE", root.join("backend.pid"))
        .env_remove("CODEX_MODEL")
        .env_remove("CODEX_REASONING_EFFORT")
        .args(["--waap-root", root.join(".state").to_str().unwrap()])
        .stdin(Stdio::null());
    command
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
root = os.path.dirname(os.environ['WAAP_TEST_PID_FILE'])
steers = 0
def emit(value):
    print(json.dumps(value), flush=True)
for line in sys.stdin:
    request = json.loads(line)
    method = request['method']
    if mode.startswith('steering'):
        with open(os.path.join(root, 'requests.ndjson'), 'a') as log:
            log.write(json.dumps(request) + '\n')
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
    if method == 'turn/steer':
        steers += 1
        result = {'turnId': 'tu_test'}
        emit({'method': 'item/agentMessage/delta', 'params': {
            'threadId': 'th_test', 'turnId': 'tu_test', 'delta': 'steered output\n'}})
    emit({'id': request['id'], 'result': result})
    if method == 'turn/interrupt':
        emit({'method': 'turn/completed', 'params': {
            'threadId': 'th_test', 'turn': {'id': 'tu_test', 'status': 'interrupted'}}})
        time.sleep(60)
    if method == 'turn/steer' and steers == 2:
        while not os.path.exists(os.path.join(root, 'release')):
            time.sleep(0.005)
        emit({'method': 'turn/completed', 'params': {
            'threadId': 'th_test', 'turn': {'id': 'tu_test', 'status': 'completed'}}})
        time.sleep(60)
    if method == 'thread/start' and mode == 'publication-failed':
        time.sleep(60)
    if method == 'turn/start' and mode.startswith('steering'):
        emit({'method': 'item/agentMessage/delta', 'params': {
            'threadId': 'th_test', 'turnId': 'tu_test', 'delta': 'native turn active\n'}})
        continue
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

struct RunningChild(Option<std::process::Child>);

impl Drop for RunningChild {
    fn drop(&mut self) {
        if let Some(child) = &mut self.0 {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

fn wait_until(mut predicate: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while !predicate() {
        assert!(Instant::now() < deadline, "condition did not become true");
        thread::sleep(Duration::from_millis(5));
    }
}

fn submit_native_steering(root: &Path, runtime: &Path, prompt: &str) {
    let output = waap_command(root, "steering", runtime)
        .args([
            "--output-format",
            "json",
            "agent",
            "steer",
            "--agent-id",
            "aa-test",
            "--prompt",
            prompt,
        ])
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    assert_eq!(
        serde_json::from_slice::<Value>(&output.stdout).unwrap(),
        json!({"agent_id": "aa-test", "accepted": true})
    );
}

#[test]
fn native_codex_steering_through_run_owner_preserves_state_output_and_cleanup() {
    for interrupt in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        init_repo(root);
        seed_backend(root);
        assert!(waap(root, &["init"], "steering").status.success());
        assert!(waap(root, &["agent", "new", "--name", "Test"], "steering")
            .status
            .success());
        let runtime = tempfile::tempdir().unwrap();
        fs::set_permissions(runtime.path(), fs::Permissions::from_mode(0o700)).unwrap();
        let mut child = RunningChild(Some(
            waap_command(root, "steering", runtime.path())
                .args(["agent", "run", "--agent-id", "aa-test", "--system", "codex"])
                .stdout(fs::File::create(root.join("run.stdout")).unwrap())
                .stderr(Stdio::piped())
                .spawn()
                .unwrap(),
        ));
        let socket = runtime
            .path()
            .join("waap/control")
            .join(root.canonicalize().unwrap().strip_prefix("/").unwrap())
            .join("agents/aa-test.sock");
        wait_until(|| {
            socket.exists()
                && fs::read_to_string(root.join("run.stdout"))
                    .unwrap()
                    .contains("native turn active")
        });
        let record_path = root.join(".state/agents/aa-test/agent.md");
        let record = fs::read(&record_path).unwrap();
        let head = git(&root.join(".state"), &["rev-parse", "HEAD"]);
        let prompt = "  exact\nλ 'quotes' \"double\" $(touch /never) & ; ";
        submit_native_steering(root, runtime.path(), prompt);
        assert_eq!(fs::read(&record_path).unwrap(), record);
        assert_eq!(git(&root.join(".state"), &["rev-parse", "HEAD"]), head);
        assert!(root.join("worktrees/aa-test").is_dir());
        if interrupt {
            assert_eq!(
                unsafe { libc::kill(child.0.as_ref().unwrap().id() as i32, libc::SIGTERM) },
                0
            );
        } else {
            submit_native_steering(root, runtime.path(), "second");
            fs::write(root.join("release"), "complete").unwrap();
        }
        wait_until(|| child.0.as_mut().unwrap().try_wait().unwrap().is_some());
        let output = child.0.take().unwrap().wait_with_output().unwrap();
        assert_eq!(output.status.success(), !interrupt, "{output:?}");
        assert!(fs::read_to_string(root.join("run.stdout"))
            .unwrap()
            .contains("steered output"));
        let record = fs::read_to_string(record_path).unwrap();
        assert!(record.contains(if interrupt {
            "status = \"failed\""
        } else {
            "status = \"completed\""
        }));
        assert!(record.contains("session_id = \"th_test\""));
        assert!(!socket.exists());
        assert!(!root.join("worktrees/aa-test").exists());
        let pid = fs::read_to_string(root.join("backend.pid")).unwrap();
        assert!(!Path::new(&format!("/proc/{pid}")).exists());
        let requests: Vec<Value> = fs::read_to_string(root.join("requests.ndjson"))
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect();
        assert!(requests.iter().all(|request| matches!(
            request["method"].as_str(),
            Some(
                "initialize"
                    | "initialized"
                    | "thread/start"
                    | "turn/start"
                    | "turn/steer"
                    | "turn/interrupt"
            )
        )));
        for method in ["initialize", "thread/start", "turn/start"] {
            assert_eq!(
                requests
                    .iter()
                    .filter(|request| request["method"] == method)
                    .count(),
                1
            );
        }
        let steering: Vec<_> = requests
            .iter()
            .filter(|request| request["method"] == "turn/steer")
            .collect();
        assert_eq!(steering.len(), if interrupt { 1 } else { 2 });
        assert_eq!(
            steering[0]["params"],
            json!({"threadId":"th_test","expectedTurnId":"tu_test",
            "input":[{"type":"text","text":prompt}]})
        );
        assert_eq!(
            requests
                .iter()
                .filter(|request| request["method"] == "turn/interrupt")
                .count(),
            usize::from(interrupt)
        );
        assert!(waap(root, &["check"], "steering").status.success());
    }
}
