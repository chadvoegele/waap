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
        if mode == 'steering-startup':
            while not os.path.exists(os.path.join(root, 'release-start')):
                time.sleep(0.005)
        result = {'turn': {'id': 'tu_test'}}
    if method == 'turn/steer':
        steers += 1
        if mode == 'steering-held':
            while not os.path.exists(os.path.join(root, 'release-steer')):
                if os.path.exists(os.path.join(root, 'complete')):
                    emit({'method': 'turn/completed', 'params': {
                        'threadId': 'th_test', 'turn': {'id': 'tu_test', 'status': 'completed'}}})
                    time.sleep(60)
                emit({'method': 'item/agentMessage/delta', 'params': {
                    'threadId': 'th_test', 'turnId': 'tu_test', 'delta': 'waiting for acknowledgement\n'}})
                time.sleep(0.01)
        if mode == 'steering-invalid-error':
            emit({'id': request['id'], 'error': None})
            continue
        if mode == 'steering-eof':
            sys.exit(0)
        if mode in ('steering-rejected', 'steering-missing'):
            code = -32601 if mode == 'steering-missing' else -32000
            emit({'id': request['id'], 'error': {'code': code, 'message': request['params']['input'][0]['text']}})
            continue
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

#[test]
fn native_codex_steering_through_run_owner_preserves_state_output_and_cleanup() {
    for interrupt in [false, true] {
        let mut run = SteeringRun::start("steering-lifecycle");
        let before = run.snapshot();
        let prompt = "  exact\nλ 'quotes' \"double\" $(touch /never) & ; ";
        let output = run.steer(prompt).output().unwrap();
        assert!(output.status.success(), "{output:?}");
        assert_eq!(
            serde_json::from_slice::<Value>(&output.stdout).unwrap(),
            json!({"agent_id":run.agent_id, "accepted":true})
        );
        assert_eq!(run.snapshot(), before);
        assert!(run
            .repo
            .path()
            .join("worktrees")
            .join(&run.agent_id)
            .is_dir());
        if interrupt {
            run.stop();
        } else {
            let output = run.steer("second").output().unwrap();
            assert!(output.status.success(), "{output:?}");
            fs::write(run.repo.path().join("release"), "complete").unwrap();
        }
        run.finish(if interrupt { "aborted" } else { "completed" }, !interrupt);
        assert!(fs::read_to_string(run.repo.path().join("run.stdout"))
            .unwrap()
            .contains("steered output"));
        let requests = run.steering_requests();
        assert_eq!(requests.len(), if interrupt { 1 } else { 2 });
        assert_eq!(
            requests[0]["params"],
            json!({"threadId":"th_test","expectedTurnId":"tu_test",
                "input":[{"type":"text","text":prompt}]})
        );
        assert_eq!(
            run.requests()
                .iter()
                .filter(|request| request["method"] == "turn/interrupt")
                .count(),
            usize::from(interrupt)
        );
        assert!(waap(run.repo.path(), &["check"], "steering")
            .status
            .success());
    }
}

struct SteeringRun {
    repo: tempfile::TempDir,
    runtime: tempfile::TempDir,
    child: RunningChild,
    mode: &'static str,
    agent_id: String,
    socket: std::path::PathBuf,
}

impl SteeringRun {
    fn start(mode: &'static str) -> Self {
        let repo = tempfile::tempdir().unwrap();
        let runtime = tempfile::tempdir().unwrap();
        let root = repo.path();
        init_repo(root);
        seed_backend(root);
        fs::set_permissions(runtime.path(), fs::Permissions::from_mode(0o700)).unwrap();
        assert!(waap(root, &["init"], mode).status.success());
        assert!(waap(root, &["agent", "new", "--name", mode], mode)
            .status
            .success());
        let agent_id = format!("aa-{mode}");
        let child = RunningChild(Some(
            waap_command(root, mode, runtime.path())
                .args(["agent", "run", "--agent-id", &agent_id, "--system", "codex"])
                .stdout(fs::File::create(root.join("run.stdout")).unwrap())
                .stderr(Stdio::piped())
                .spawn()
                .unwrap(),
        ));
        use std::os::unix::ffi::OsStrExt;
        let digest = ring::digest::digest(
            &ring::digest::SHA256,
            root.canonicalize().unwrap().as_os_str().as_bytes(),
        );
        let hash: String = digest.as_ref()[..8]
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect();
        let socket = runtime
            .path()
            .join("waap")
            .join(hash)
            .join(format!("{agent_id}.sock"));
        wait_until(|| {
            socket.exists()
                && if mode == "steering-startup" {
                    fs::read_to_string(root.join("requests.ndjson"))
                        .unwrap_or_default()
                        .contains("turn/start")
                } else {
                    fs::read_to_string(root.join("run.stdout"))
                        .unwrap()
                        .contains("native turn active")
                }
        });
        Self {
            repo,
            runtime,
            child,
            mode,
            agent_id,
            socket,
        }
    }

    fn steer(&self, prompt: &str) -> Command {
        let mut command = waap_command(self.repo.path(), self.mode, self.runtime.path());
        command.args([
            "--output-format",
            "json",
            "agent",
            "steer",
            "--agent-id",
            &self.agent_id,
            "--prompt",
            prompt,
        ]);
        command
    }

    fn snapshot(&self) -> (Vec<u8>, String, String) {
        let state = self.repo.path().join(".state");
        (
            fs::read(state.join(format!("agents/{}/agent.md", self.agent_id))).unwrap(),
            git(&state, &["rev-parse", "HEAD"]),
            git(&state, &["status", "--porcelain"]),
        )
    }

    fn requests(&self) -> Vec<Value> {
        fs::read_to_string(self.repo.path().join("requests.ndjson"))
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect()
    }

    fn steering_requests(&self) -> Vec<Value> {
        self.requests()
            .into_iter()
            .filter(|request| request["method"] == "turn/steer")
            .collect()
    }

    fn stop(&self) {
        let stopped = waap_command(self.repo.path(), self.mode, self.runtime.path())
            .args(["agent", "stop", "--agent-id", &self.agent_id])
            .output()
            .unwrap();
        assert!(stopped.status.success(), "{stopped:?}");
    }

    fn finish(&mut self, status: &str, success: bool) {
        wait_until(|| self.child.0.as_mut().unwrap().try_wait().unwrap().is_some());
        let output = self.child.0.take().unwrap().wait_with_output().unwrap();
        assert_eq!(output.status.success(), success, "{output:?}");
        let snapshot = self.snapshot();
        let record = String::from_utf8(snapshot.0.clone()).unwrap();
        assert!(
            record.contains(&format!("status = \"{status}\"")),
            "{record}"
        );
        assert!(record.contains("session_id = \"th_test\""));
        assert!(
            snapshot.2.is_empty(),
            "unexpected persisted files: {snapshot:?}"
        );
        assert!(!self.socket.exists());
        assert!(!self
            .repo
            .path()
            .join("worktrees")
            .join(&self.agent_id)
            .exists());
        assert!(!git(self.repo.path(), &["worktree", "list"]).contains(&self.agent_id));
        let pid = fs::read_to_string(self.repo.path().join("backend.pid")).unwrap();
        assert!(!Path::new(&format!("/proc/{pid}")).exists());
        let later = self.steer("after exit").output().unwrap();
        assert!(!later.status.success(), "{later:?}");
        assert!(String::from_utf8_lossy(&later.stderr).contains("expected running"));
        assert_eq!(self.snapshot(), snapshot);
        for method in ["initialize", "thread/start", "turn/start"] {
            assert_eq!(
                self.requests()
                    .iter()
                    .filter(|request| request["method"] == method)
                    .count(),
                1,
            );
        }
        assert!(self.requests().iter().all(|request| matches!(
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
    }
}

#[test]
fn startup_steering_is_rejected_without_deferring_delivery_to_the_active_turn() {
    let mut run = SteeringRun::start("steering-startup");
    let before = run.snapshot();
    let output = run.steer("too early").output().unwrap();
    assert!(!output.status.success(), "{output:?}");
    assert!(String::from_utf8_lossy(&output.stderr).contains("no_active_turn"));
    assert_eq!(run.snapshot(), before);
    fs::write(run.repo.path().join("release-start"), "activate").unwrap();
    wait_until(|| {
        fs::read_to_string(run.repo.path().join("run.stdout"))
            .unwrap()
            .contains("native turn active")
    });
    run.stop();
    run.finish("aborted", false);
    assert!(run.steering_requests().is_empty());
}

#[test]
fn native_failures_reach_cli_without_prompt_disclosure_retries_or_lifecycle_changes() {
    for (mode, diagnostic) in [
        ("steering-rejected", "backend_rejected"),
        ("steering-missing", "unsupported_backend_version"),
        ("steering-eof", "acceptance is unknown"),
        ("steering-invalid-error", "acceptance is unknown"),
    ] {
        let mut run = SteeringRun::start(mode);
        let before = run.snapshot();
        let prompt = "secret-should-not-appear-in-diagnostics";
        let output = run.steer(prompt).output().unwrap();
        assert!(!output.status.success(), "{output:?}");
        assert!(output.stdout.is_empty());
        let stderr = String::from_utf8(output.stderr).unwrap();
        assert!(stderr.contains(diagnostic), "{stderr}");
        assert!(!stderr.contains(prompt), "{stderr}");
        assert_eq!(run.steering_requests().len(), 1);
        assert_eq!(
            run.steering_requests()[0]["params"],
            json!({
                "threadId":"th_test", "expectedTurnId":"tu_test",
                "input":[{"type":"text", "text":prompt}]
            })
        );
        if mode == "steering-eof" {
            run.finish("failed", false);
        } else {
            assert_eq!(run.snapshot(), before);
            assert!(run.socket.exists());
            assert!(run
                .repo
                .path()
                .join("worktrees")
                .join(&run.agent_id)
                .is_dir());
            run.stop();
            run.finish("aborted", false);
        }
    }
}

#[test]
fn concurrent_cli_call_is_busy_while_native_ack_and_run_output_continue() {
    let mut run = SteeringRun::start("steering-held");
    let before = run.snapshot();
    let mut first = RunningChild(Some(
        run.steer("first").stdout(Stdio::piped()).spawn().unwrap(),
    ));
    wait_until(|| run.steering_requests().len() == 1);
    let mut second = RunningChild(Some(
        run.steer("second")
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap(),
    ));
    wait_until(|| {
        fs::read_to_string(run.repo.path().join("run.stdout"))
            .unwrap()
            .contains("waiting for acknowledgement")
    });
    assert!(first.0.as_mut().unwrap().try_wait().unwrap().is_none());
    wait_until(|| second.0.as_mut().unwrap().try_wait().unwrap().is_some());
    let rejected = second.0.take().unwrap().wait_with_output().unwrap();
    assert!(!rejected.status.success());
    assert!(String::from_utf8_lossy(&rejected.stderr).contains("busy"));
    assert!(first.0.as_mut().unwrap().try_wait().unwrap().is_none());
    assert_eq!(
        run.steering_requests().len(),
        1,
        "only one native dispatch may be active"
    );
    assert_eq!(run.snapshot(), before);
    fs::write(run.repo.path().join("release-steer"), "acknowledge").unwrap();
    wait_until(|| first.0.as_mut().unwrap().try_wait().unwrap().is_some());
    let output = first.0.take().unwrap().wait_with_output().unwrap();
    assert!(output.status.success(), "{output:?}");
    assert_eq!(
        serde_json::from_slice::<Value>(&output.stdout).unwrap(),
        json!({"agent_id":run.agent_id, "accepted":true})
    );
    let next = run.steer("after acknowledgement").output().unwrap();
    assert!(next.status.success(), "{next:?}");
    let requests = run.steering_requests();
    assert_eq!(requests.len(), 2);
    assert_eq!(requests[0]["params"]["input"][0]["text"], "first");
    assert_eq!(
        requests[1]["params"]["input"][0]["text"],
        "after acknowledgement"
    );
    assert_ne!(requests[0]["id"], requests[1]["id"]);
    assert_eq!(run.snapshot(), before);
    fs::write(run.repo.path().join("release"), "complete").unwrap();
    run.finish("completed", true);
}

#[test]
fn completion_during_native_submission_reports_uncertainty_without_retry() {
    let mut run = SteeringRun::start("steering-held");
    let mut active = RunningChild(Some(
        run.steer("active").stderr(Stdio::piped()).spawn().unwrap(),
    ));
    wait_until(|| run.steering_requests().len() == 1);
    let mut pending = RunningChild(Some(
        run.steer("pending").stderr(Stdio::piped()).spawn().unwrap(),
    ));
    fs::write(
        run.repo.path().join("complete"),
        "exit before acknowledgement",
    )
    .unwrap();
    wait_until(|| active.0.as_mut().unwrap().try_wait().unwrap().is_some());
    let output = active.0.take().unwrap().wait_with_output().unwrap();
    assert!(!output.status.success(), "{output:?}");
    assert!(String::from_utf8_lossy(&output.stderr).contains("acceptance is unknown"));
    wait_until(|| pending.0.as_mut().unwrap().try_wait().unwrap().is_some());
    let pending = pending.0.take().unwrap().wait_with_output().unwrap();
    assert!(!pending.status.success(), "{pending:?}");
    assert_eq!(
        run.steering_requests().len(),
        1,
        "pending calls must not dispatch after completion"
    );
    run.finish("completed", true);
}
