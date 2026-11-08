mod common;

use std::fs;
use std::io::{ErrorKind, Write};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use tempfile::{tempdir, TempDir};

use common::{git, init_repo, isolate_git_config};

struct Project {
    _temp: TempDir,
    root: PathBuf,
    state: PathBuf,
}

impl Project {
    fn new() -> Self {
        let temp = tempdir().unwrap();
        let root = temp.path().join("repo");
        fs::create_dir(&root).unwrap();
        init_repo(&root);
        let runtime = temp.path().join("runtime");
        fs::create_dir(&runtime).unwrap();
        fs::set_permissions(&runtime, fs::Permissions::from_mode(0o700)).unwrap();
        let state = root.join(".state");
        let project = Self {
            _temp: temp,
            root,
            state,
        };
        let output = project.run_command("", &["init"]);
        assert_success(&output);
        project
    }

    fn build_command(&self) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_waap"));
        isolate_git_config(&mut command);
        command
            .current_dir(&self.root)
            .env("XDG_RUNTIME_DIR", self._temp.path().join("runtime"))
            .arg("--waap-root")
            .arg(&self.state);
        command
    }

    fn run_command(&self, stdin: &str, args: &[&str]) -> Output {
        let mut child = self
            .build_command()
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let result = child.stdin.take().unwrap().write_all(stdin.as_bytes());
        if let Err(error) = result {
            assert_eq!(error.kind(), ErrorKind::BrokenPipe);
        }
        child.wait_with_output().unwrap()
    }

    fn create_agent(&self, name: &str) -> String {
        let output = self.run_command(
            "Complete the deterministic fixture.\n",
            &["--output-format", "json", "agent", "new", "--name", name],
        );
        assert_success(&output);
        serde_json::from_slice::<serde_json::Value>(&output.stdout).unwrap()["agent_id"]
            .as_str()
            .unwrap()
            .to_string()
    }

    fn get_agent_record_path(&self, agent_id: &str) -> PathBuf {
        self.state.join("agents").join(agent_id).join("agent.md")
    }

    fn get_socket_path(&self, agent_id: &str) -> PathBuf {
        use std::os::unix::ffi::OsStrExt;
        let digest = ring::digest::digest(
            &ring::digest::SHA256,
            self.root.canonicalize().unwrap().as_os_str().as_bytes(),
        );
        let hash: String = digest.as_ref()[..8]
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect();
        self._temp
            .path()
            .join("runtime/waap")
            .join(hash)
            .join(format!("{agent_id}.sock"))
    }

    fn get_worktree_path(&self, agent_id: &str) -> PathBuf {
        self.root.join("worktrees").join(agent_id)
    }
}

fn assert_success(output: &Output) {
    assert!(
        output.status.success(),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

fn write_executable(path: &Path, body: &str) {
    fs::write(path, body).unwrap();
    let mut permissions = fs::metadata(path).unwrap().permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(path, permissions).unwrap();
}

fn write_pi_script(path: &Path, body: &str) {
    fs::write(
        path,
        format!("set -eu\nprintf '%s' \"$$\" > \"$WAAP_FAKE_PID\"\n{body}\n"),
    )
    .unwrap();
}

fn build_shutdown_notification_script() -> String {
    let record = serde_json::json!({
        "type": "extension_ui_request",
        "method": "notify",
        "message": "shutdown-notification".to_owned() + &"x".repeat(128 * 1024),
    });
    format!("printf '%s\\n' '{record}'\n")
}

fn get_fake_pi_runner_path() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/fake-pi-runner")
}

fn assert_reaped(pid_file: &Path) {
    let pid = fs::read_to_string(pid_file).unwrap();
    assert!(!Path::new("/proc").join(pid).exists());
}

fn wait_for(description: &str, condition: impl Fn() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while !condition() {
        assert!(
            Instant::now() < deadline,
            "timed out waiting for {description}"
        );
        thread::sleep(Duration::from_millis(20));
    }
}

fn wait_output(mut child: Child) -> Output {
    let deadline = Instant::now() + Duration::from_secs(10);
    while child.try_wait().unwrap().is_none() {
        if Instant::now() >= deadline {
            child.kill().unwrap();
            let output = child.wait_with_output().unwrap();
            panic!("owner failed to exit: {output:?}");
        }
        thread::sleep(Duration::from_millis(10));
    }
    child.wait_with_output().unwrap()
}

fn assert_record(project: &Project, agent_id: &str, status: &str, system: &str, session: &str) {
    let record = fs::read_to_string(project.get_agent_record_path(agent_id)).unwrap();
    assert!(
        record.contains(&format!("status = \"{status}\"")),
        "{record}"
    );
    assert!(
        record.contains(&format!("system = \"{system}\"")),
        "{record}"
    );
    assert!(
        record.contains(&format!("session_id = \"{session}\"")),
        "{record}"
    );
}

#[test]
fn test_omitted_system_runs_fake_pi_after_authentic_session_is_persisted() {
    let project = Project::new();
    let agent_id = project.create_agent("pi-process-success");
    let fake = project.root.join("fake-pi-success");
    let pid = project.root.join("fake-pi-success.pid");
    let snapshot = project.root.join("prompt-state");
    write_pi_script(
        &fake,
        r#"IFS= read -r state
printf '%s\n' '{"type":"response","id":"waap-1","command":"get_state","success":true,"data":{"sessionId":"pi-process-session"}}'
IFS= read -r prompt
cp "$WAAP_FAKE_RECORD" "$WAAP_FAKE_SNAPSHOT"
printf '%s\n' "$prompt" >> "$WAAP_FAKE_SNAPSHOT"
printf '%s\n' '{"type":"response","id":"waap-2","command":"prompt","success":true,"data":{"disposition":"started"}}'
printf '%s\n' '{"type":"message_update","assistantMessageEvent":{"type":"text_delta","delta":"fake-pi-output"}}'
printf '%s\n' '{"type":"message_end","message":{"role":"assistant","stopReason":"stop"}}'
printf '%s\n' '{"type":"agent_settled"}'
while IFS= read -r ignored; do :; done
"#,
    );

    let output = project
        .build_command()
        .args(["agent", "run", "--agent-id", &agent_id])
        .env("WAAP_PI_BIN", get_fake_pi_runner_path())
        .env("WAAP_FAKE_PI_SCRIPT", &fake)
        .env("WAAP_FAKE_PID", &pid)
        .env("WAAP_FAKE_RECORD", project.get_agent_record_path(&agent_id))
        .env("WAAP_FAKE_SNAPSHOT", &snapshot)
        .output()
        .unwrap();

    assert_success(&output);
    assert!(String::from_utf8_lossy(&output.stdout).contains("fake-pi-output"));
    assert_record(&project, &agent_id, "completed", "pi", "pi-process-session");
    let prompt_state = fs::read_to_string(snapshot).unwrap();
    assert!(prompt_state.contains("status = \"running\""));
    assert!(prompt_state.contains("system = \"pi\""));
    assert!(prompt_state.contains("session_id = \"pi-process-session\""));
    assert!(prompt_state.contains("Complete when instructions in"));
    assert!(!project.get_worktree_path(&agent_id).exists());
    let subjects = git(&project.state, &["log", "--pretty=%s"]);
    assert!(subjects.contains(&format!("waap agent run {agent_id}")));
    assert!(subjects.contains(&format!("waap agent pi session {agent_id}")));
    assert!(subjects.contains(&format!("waap agent completed {agent_id}")));
    assert_reaped(&pid);
}

#[test]
fn test_fake_pi_failure_exits_nonzero_persists_failed_and_cleans_worktree() {
    let project = Project::new();
    let agent_id = project.create_agent("pi-process-failure");
    let fake = project.root.join("fake-pi-failure");
    let pid = project.root.join("fake-pi-failure.pid");
    write_pi_script(
        &fake,
        r#"IFS= read -r state
printf '%s\n' '{"type":"response","id":"waap-1","command":"get_state","success":true,"data":{"sessionId":"pi-failed-session"}}'
IFS= read -r prompt
printf '%s\n' '{"type":"response","id":"waap-2","command":"prompt","success":true,"data":{"disposition":"started"}}'
printf '%s\n' '{"type":"message_end","message":{"role":"assistant","stopReason":"error"}}'
printf '%s\n' '{"type":"agent_settled"}'
while IFS= read -r ignored; do :; done
"#,
    );

    let output = project
        .build_command()
        .args(["agent", "run", "--system", "pi", "--agent-id", &agent_id])
        .env("WAAP_PI_BIN", get_fake_pi_runner_path())
        .env("WAAP_FAKE_PI_SCRIPT", fake)
        .env("WAAP_FAKE_PID", &pid)
        .output()
        .unwrap();

    assert!(!output.status.success());
    assert_record(&project, &agent_id, "failed", "pi", "pi-failed-session");
    assert!(!project.get_worktree_path(&agent_id).exists());
    assert_reaped(&pid);
}

#[test]
fn test_stop_uses_shared_socket_after_steering_and_owner_persists_aborted() {
    let project = Project::new();
    let agent_id = project.create_agent("pi-process-abort");
    let fake = project.root.join("fake-pi-abort");
    let pid = project.root.join("fake-pi-abort.pid");
    let log = project.root.join("pi-commands");
    let ready = project.root.join("pi-ready");
    write_pi_script(
        &fake,
        &(r#"while IFS= read -r line; do
  printf '%s\n' "$line" >> "$WAAP_FAKE_LOG"
  case "$line" in
    *'"type":"get_state"'*)
      printf '%s\n' '{"type":"response","id":"waap-1","command":"get_state","success":true,"data":{"sessionId":"pi-abort-session"}}'
      ;;
    *'"type":"prompt"'*)
      printf '%s\n' '{"type":"response","id":"waap-2","command":"prompt","success":true,"data":{"disposition":"started"}}'
      touch "$WAAP_FAKE_READY"
      ;;
    *'"type":"steer"'*)
      printf '%s\n' '{"type":"response","id":"waap-3","command":"steer","success":true,"data":{"disposition":"queued"}}'
      ;;
    *'"type":"clear_queue"'*)
      printf '%s\n' '{"type":"response","id":"waap-4","command":"clear_queue","success":true}'
      ;;
    *'"type":"abort"'*)
      printf '%s\n' '{"type":"message_end","message":{"role":"assistant","stopReason":"aborted"}}'
      printf '%s\n' '{"type":"agent_settled"}'
      printf '%s\n' '{"type":"response","id":"waap-5","command":"abort","success":true}'
      ;;
  esac
done
"#.to_owned() + &build_shutdown_notification_script()),
    );
    let runner = project
        .build_command()
        .args(["agent", "run", "--system", "pi", "--agent-id", &agent_id])
        .env("WAAP_PI_BIN", get_fake_pi_runner_path())
        .env("WAAP_FAKE_PI_SCRIPT", fake)
        .env("WAAP_FAKE_PID", &pid)
        .env("WAAP_FAKE_LOG", &log)
        .env("WAAP_FAKE_READY", &ready)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    wait_for("Pi prompt acceptance", || ready.exists());

    let prompt = "  雪\n'quoted' $(touch injected) `pwd`  ";
    let before_record = fs::read(project.get_agent_record_path(&agent_id)).unwrap();
    let steer = project.run_command(
        "",
        &[
            "agent",
            "steer",
            "--agent-id",
            &agent_id,
            "--prompt",
            prompt,
        ],
    );
    assert_success(&steer);
    assert_eq!(
        fs::read(project.get_agent_record_path(&agent_id)).unwrap(),
        before_record
    );
    let stop = project.run_command("", &["agent", "stop", "--agent-id", &agent_id]);
    assert_success(&stop);
    let owner = wait_output(runner);

    assert_success(&owner);
    assert!(!String::from_utf8_lossy(&owner.stdout).contains("shutdown-notification"));
    assert!(!String::from_utf8_lossy(&owner.stderr).contains("transition"));
    assert_record(&project, &agent_id, "aborted", "pi", "pi-abort-session");
    assert_eq!(
        fs::read_to_string(&log)
            .unwrap()
            .matches("\"type\":\"abort\"")
            .count(),
        1
    );
    let commands = fs::read_to_string(&log).unwrap();
    let records: Vec<serde_json::Value> = commands
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    let steer = records
        .iter()
        .find(|record| record["type"] == "steer")
        .unwrap();
    assert_eq!(steer["message"], prompt);
    assert_eq!(
        records
            .iter()
            .filter(|record| record["type"] == "clear_queue")
            .count(),
        1
    );
    assert!(
        records
            .iter()
            .position(|record| record["type"] == "clear_queue")
            .unwrap()
            < records
                .iter()
                .position(|record| record["type"] == "abort")
                .unwrap()
    );
    assert!(!project.root.join("injected").exists());
    assert!(!project.get_worktree_path(&agent_id).exists());
    assert!(!project.get_socket_path(&agent_id).exists());
    assert_reaped(&pid);
}

#[test]
fn test_stop_rejects_running_record_without_system_without_mutation() {
    let project = Project::new();
    let agent_id = project.create_agent("legacy-sessionless-system");
    let record_path = project.get_agent_record_path(&agent_id);
    let ready = fs::read_to_string(&record_path).unwrap();
    fs::write(
        &record_path,
        ready.replace(
            "status = \"ready\"",
            "status = \"running\"\nsession_id = \"legacy-session\"",
        ),
    )
    .unwrap();
    git(&project.state, &["add", "agents"]);
    git(
        &project.state,
        &["commit", "-q", "-m", "seed legacy running record"],
    );
    let before_record = fs::read(&record_path).unwrap();
    let before_head = git(&project.state, &["rev-parse", "HEAD"]);

    let output = project.run_command("", &["agent", "stop", "--agent-id", &agent_id]);

    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("no persisted system"));
    assert_eq!(fs::read(record_path).unwrap(), before_record);
    assert_eq!(git(&project.state, &["rev-parse", "HEAD"]), before_head);
}

#[test]
fn test_codex_shared_control_stop_keeps_owner_outcome_and_cleanup() {
    let project = Project::new();
    let agent_id = project.create_agent("codex-process-abort");
    let bin = project.root.join("fake-bin");
    fs::create_dir(&bin).unwrap();
    let fake = bin.join("codex");
    let log = project.root.join("codex-commands");
    let ready = project.root.join("codex-ready");
    write_executable(
        &fake,
        r#"#!/usr/bin/env bash
set -eu
while IFS= read -r line; do
  printf '%s\n' "$line" >> "$WAAP_FAKE_LOG"
  case "$line" in
    *'"method":"initialize"'*) printf '%s\n' '{"id":0,"result":{}}' ;;
    *'"method":"thread/start"'*) printf '%s\n' '{"id":1,"result":{"thread":{"id":"codex-abort-thread"}}}' ;;
    *'"method":"turn/start"'*)
      printf '%s\n' '{"id":2,"result":{"turn":{"id":"codex-abort-turn"}}}'
      touch "$WAAP_FAKE_READY"
      while true; do
        if IFS= read -r -t 0.05 control; then
          printf '%s\n' "$control" >> "$WAAP_FAKE_LOG"
          if [[ "$control" == *'"method":"turn/interrupt"'* ]]; then
            printf '%s\n' '{"id":3,"result":{}}'
            printf '%s\n' '{"method":"turn/completed","params":{"threadId":"codex-abort-thread","turn":{"id":"codex-abort-turn","status":"interrupted"}}}'
            exit 0
          fi
        else
          printf '%s\n' '{"method":"heartbeat","params":{}}'
        fi
      done
      ;;
  esac
done
"#,
    );
    let path = format!(
        "{}:{}",
        bin.display(),
        std::env::var("PATH").unwrap_or_default()
    );
    let runner = project
        .build_command()
        .args(["agent", "run", "--agent-id", &agent_id, "--system", "codex"])
        .env("PATH", path)
        .env("WAAP_FAKE_LOG", &log)
        .env("WAAP_FAKE_READY", &ready)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    wait_for("Codex turn start", || ready.exists());

    let stop = project.run_command("", &["agent", "stop", "--agent-id", &agent_id]);
    assert_success(&stop);
    let owner = wait_output(runner);

    assert_success(&owner);
    assert!(!String::from_utf8_lossy(&owner.stderr).contains("transition"));
    assert_record(
        &project,
        &agent_id,
        "aborted",
        "codex",
        "codex-abort-thread",
    );
    assert_eq!(
        fs::read_to_string(log)
            .unwrap()
            .matches("\"method\":\"turn/interrupt\"")
            .count(),
        1
    );
    assert!(!project.get_worktree_path(&agent_id).exists());
}

#[test]
fn test_handled_prompt_with_or_without_extension_work_fails_and_reaps() {
    for extension_work in [false, true] {
        let project = Project::new();
        let agent_id = project.create_agent("handled");
        let fake = project.root.join("handled-pi");
        let pid = project.root.join("handled.pid");
        write_pi_script(
            &fake,
            &format!(
                r#"
IFS= read -r state
printf '%s\n' '{{"type":"response","id":"waap-1","command":"get_state","success":true,"data":{{"sessionId":"handled-session"}}}}'
IFS= read -r prompt
{}
printf '%s\n' '{{"type":"response","id":"waap-2","command":"prompt","success":true,"data":{{"disposition":"handled"}}}}'
while IFS= read -r ignored; do :; done
"#,
                if extension_work {
                    "printf '%s\\n' '{\"type\":\"agent_start\"}'"
                } else {
                    ""
                }
            ),
        );
        let output = wait_output(
            project
                .build_command()
                .args(["agent", "run", "--agent-id", &agent_id])
                .env("WAAP_PI_BIN", get_fake_pi_runner_path())
                .env("WAAP_FAKE_PI_SCRIPT", fake)
                .env("WAAP_FAKE_PID", &pid)
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .unwrap(),
        );
        assert!(!output.status.success());
        assert!(String::from_utf8_lossy(&output.stderr).contains("handled"));
        assert_record(&project, &agent_id, "failed", "pi", "handled-session");
        assert!(!project.get_worktree_path(&agent_id).exists());
        assert_reaped(&pid);
    }
}

#[test]
fn test_post_eof_protocol_output_larger_than_pipe_capacity_preserves_owner_outcome() {
    for (reason, status) in [("stop", "completed"), ("error", "failed")] {
        let project = Project::new();
        let agent_id = project.create_agent("shutdown-output");
        let fake = project.root.join("shutdown-pi");
        let pid = project.root.join("shutdown-pi.pid");
        let script = r#"
IFS= read -r state
printf '%s\n' '{"type":"response","id":"waap-1","command":"get_state","success":true,"data":{"sessionId":"shutdown-session"}}'
IFS= read -r prompt
printf '%s\n' '{"type":"response","id":"waap-2","command":"prompt","success":true,"data":{"disposition":"started"}}'
printf '%s\n' '{"type":"message_end","message":{"role":"assistant","stopReason":"STOP_REASON"}}'
printf '%s\n' '{"type":"agent_settled"}'
while IFS= read -r ignored; do :; done
"#.replace("STOP_REASON", reason) + &build_shutdown_notification_script();
        write_pi_script(&fake, &script);
        let output = wait_output(
            project
                .build_command()
                .args(["agent", "run", "--agent-id", &agent_id])
                .env("WAAP_PI_BIN", get_fake_pi_runner_path())
                .env("WAAP_FAKE_PI_SCRIPT", fake)
                .env("WAAP_FAKE_PID", &pid)
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .unwrap(),
        );
        assert_eq!(output.status.success(), status == "completed", "{output:?}");
        assert!(!String::from_utf8_lossy(&output.stdout).contains("shutdown-notification"));
        assert!(!String::from_utf8_lossy(&output.stderr).contains("Pi RPC did not exit"));
        assert_record(&project, &agent_id, status, "pi", "shutdown-session");
        assert_reaped(&pid);
        assert!(!project.get_worktree_path(&agent_id).exists());
        assert!(!project.get_socket_path(&agent_id).exists());
    }
}

#[test]
fn test_shutdown_protocol_errors_and_nonzero_exit_fail_and_reap_owner_child() {
    for (shutdown, error) in [
        ("exec 1>&-; sleep 0.1; exit 7", "Pi RPC exited with status"),
        ("printf 'not-json\\n'; exec sleep 30", "malformed JSON"),
        (
            "printf '{\"type\":\"partial\"}'; exec 1>&-; exec sleep 30",
            "unterminated JSONL record",
        ),
    ] {
        let project = Project::new();
        let agent_id = project.create_agent("shutdown-error");
        let fake = project.root.join("shutdown-error-pi");
        let pid = project.root.join("shutdown-error-pi.pid");
        let script = r#"
IFS= read -r state
printf '%s\n' '{"type":"response","id":"waap-1","command":"get_state","success":true,"data":{"sessionId":"shutdown-error-session"}}'
IFS= read -r prompt
printf '%s\n' '{"type":"response","id":"waap-2","command":"prompt","success":true,"data":{"disposition":"started"}}'
printf '%s\n' '{"type":"message_end","message":{"role":"assistant","stopReason":"stop"}}'
printf '%s\n' '{"type":"agent_settled"}'
while IFS= read -r ignored; do :; done
"#.to_owned() + shutdown;
        write_pi_script(&fake, &script);
        let output = wait_output(
            project
                .build_command()
                .args(["agent", "run", "--agent-id", &agent_id])
                .env("WAAP_PI_BIN", get_fake_pi_runner_path())
                .env("WAAP_FAKE_PI_SCRIPT", fake)
                .env("WAAP_FAKE_PID", &pid)
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .unwrap(),
        );
        assert!(!output.status.success());
        assert!(
            String::from_utf8_lossy(&output.stderr).contains(error),
            "{output:?}"
        );
        assert_record(
            &project,
            &agent_id,
            "failed",
            "pi",
            "shutdown-error-session",
        );
        assert_reaped(&pid);
        assert!(!project.get_worktree_path(&agent_id).exists());
        assert!(!project.get_socket_path(&agent_id).exists());
    }
}

#[test]
fn test_descendant_holding_native_stdout_does_not_block_owner_cleanup() {
    let project = Project::new();
    let agent_id = project.create_agent("descendant");
    let fake = project.root.join("descendant-pi");
    let pid = project.root.join("descendant-pi.pid");
    let descendant = project.root.join("descendant.pid");
    write_pi_script(
        &fake,
        r#"
IFS= read -r state
printf '%s\n' '{"type":"response","id":"waap-1","command":"get_state","success":true,"data":{"sessionId":"descendant-session"}}'
IFS= read -r prompt
sleep 30 < /dev/null 2>/dev/null &
printf '%s' "$!" > "$WAAP_DESCENDANT_PID"
printf '%s\n' '{"type":"response","id":"waap-2","command":"prompt","success":true,"data":{"disposition":"started"}}'
printf '%s\n' '{"type":"message_end","message":{"role":"assistant","stopReason":"stop"}}'
printf '%s\n' '{"type":"agent_settled"}'
while IFS= read -r ignored; do :; done
"#,
    );
    let runner = project
        .build_command()
        .args(["agent", "run", "--agent-id", &agent_id])
        .env("WAAP_PI_BIN", get_fake_pi_runner_path())
        .env("WAAP_FAKE_PI_SCRIPT", fake)
        .env("WAAP_FAKE_PID", &pid)
        .env("WAAP_DESCENDANT_PID", &descendant)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let output = wait_output(runner);
    let descendant_id = fs::read_to_string(descendant).unwrap();
    let alive = Path::new("/proc").join(&descendant_id).exists();
    let _ = Command::new("kill")
        .args(["-KILL", &descendant_id])
        .status();
    assert!(
        alive,
        "owner should exit while native stdout is still held open"
    );
    assert_success(&output);
    assert_record(&project, &agent_id, "completed", "pi", "descendant-session");
    assert_reaped(&pid);
    assert!(!project.get_worktree_path(&agent_id).exists());
}

#[test]
fn test_startup_protocol_failures_reap_child_without_submitting_task_work() {
    for response in [
        "{\"type\":\"response\",\"id\":\"waap-1\",\"command\":\"get_state\",\"success\":true,\"data\":{}}",
        "not-json",
        "",
    ] {
        let project = Project::new();
        let agent_id = project.create_agent("startup-error");
        let fake = project.root.join("startup-pi");
        let pid = project.root.join("startup-pi.pid");
        write_pi_script(&fake, &format!("IFS= read -r state\nprintf '%s\\n' '{response}'\n"));
        let output = wait_output(project.build_command().args(["agent", "run", "--agent-id", &agent_id])
            .env("WAAP_PI_BIN", get_fake_pi_runner_path()).env("WAAP_FAKE_PI_SCRIPT", fake).env("WAAP_FAKE_PID", &pid)
            .stdout(Stdio::piped()).stderr(Stdio::piped()).spawn().unwrap());
        assert!(!output.status.success());
        let record = fs::read_to_string(project.get_agent_record_path(&agent_id)).unwrap();
        assert!(record.contains("status = \"failed\""));
        assert!(!record.contains("session_id"));
        assert!(!project.get_worktree_path(&agent_id).exists());
        assert!(!project.get_socket_path(&agent_id).exists());
        assert_reaped(&pid);
    }
}

#[test]
fn test_socket_stop_acknowledgement_loss_remains_unknown_after_settlement() {
    let project = Project::new();
    let agent_id = project.create_agent("unknown-stop");
    let fake = project.root.join("unknown-pi");
    let pid = project.root.join("unknown-pi.pid");
    let ready = project.root.join("ready");
    let log = project.root.join("commands");
    write_pi_script(
        &fake,
        r#"
while IFS= read -r line; do
  printf '%s\n' "$line" >> "$WAAP_FAKE_LOG"
  case "$line" in
    *'"type":"get_state"'*) printf '%s\n' '{"type":"response","id":"waap-1","command":"get_state","success":true,"data":{"sessionId":"unknown-session"}}' ;;
    *'"type":"prompt"'*)
      printf '%s\n' '{"type":"response","id":"waap-2","command":"prompt","success":true,"data":{"disposition":"started"}}'
      touch "$WAAP_FAKE_READY" ;;
    *'"type":"clear_queue"'*) printf '%s\n' '{"type":"response","id":"waap-3","command":"clear_queue","success":true}' ;;
    *'"type":"abort"'*)
      printf '%s\n' '{"type":"message_end","message":{"role":"assistant","stopReason":"aborted"}}'
      printf '%s\n' '{"type":"agent_settled"}'
      exit 0 ;;
  esac
done
"#,
    );
    let runner = project
        .build_command()
        .args(["agent", "run", "--agent-id", &agent_id])
        .env("WAAP_PI_BIN", get_fake_pi_runner_path())
        .env("WAAP_FAKE_PI_SCRIPT", fake)
        .env("WAAP_FAKE_PID", &pid)
        .env("WAAP_FAKE_READY", &ready)
        .env("WAAP_FAKE_LOG", &log)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    wait_for("Pi prompt", || ready.exists());
    let stopped = project.run_command("", &["agent", "stop", "--agent-id", &agent_id]);
    assert!(!stopped.status.success());
    let diagnostic = String::from_utf8_lossy(&stopped.stderr);
    assert!(diagnostic.contains("unknown"), "{diagnostic}");
    assert!(diagnostic.contains("no automatic retry"), "{diagnostic}");
    let owner = wait_output(runner);
    assert!(!owner.status.success());
    assert_record(&project, &agent_id, "failed", "pi", "unknown-session");
    assert_eq!(
        fs::read_to_string(log)
            .unwrap()
            .matches("\"type\":\"abort\"")
            .count(),
        1
    );
    assert!(!project.get_socket_path(&agent_id).exists());
    assert!(!project.get_worktree_path(&agent_id).exists());
    assert_reaped(&pid);
}

#[test]
fn test_signals_and_stop_all_share_the_native_clear_abort_sequence() {
    for operation in ["-INT", "-TERM", "all"] {
        let project = Project::new();
        let agent_id = project.create_agent("signal");
        let fake = project.root.join("signal-pi");
        let pid = project.root.join("signal-pi.pid");
        let ready = project.root.join("ready");
        let log = project.root.join("commands");
        write_pi_script(
            &fake,
            r#"
while IFS= read -r line; do
  printf '%s\n' "$line" >> "$WAAP_FAKE_LOG"
  case "$line" in
    *'"type":"get_state"'*) printf '%s\n' '{"type":"response","id":"waap-1","command":"get_state","success":true,"data":{"sessionId":"signal-session"}}' ;;
    *'"type":"prompt"'*)
      printf '%s\n' '{"type":"response","id":"waap-2","command":"prompt","success":true,"data":{"disposition":"started"}}'
      touch "$WAAP_FAKE_READY" ;;
    *'"type":"clear_queue"'*) printf '%s\n' '{"type":"response","id":"waap-3","command":"clear_queue","success":true}' ;;
    *'"type":"abort"'*)
      printf '%s\n' '{"type":"agent_settled"}'
      printf '%s\n' '{"type":"response","id":"waap-4","command":"abort","success":true}' ;;
  esac
done
"#,
        );
        let runner = project
            .build_command()
            .args(["agent", "run", "--agent-id", &agent_id])
            .env("WAAP_PI_BIN", get_fake_pi_runner_path())
            .env("WAAP_FAKE_PI_SCRIPT", fake)
            .env("WAAP_FAKE_PID", &pid)
            .env("WAAP_FAKE_READY", &ready)
            .env("WAAP_FAKE_LOG", &log)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        wait_for("Pi prompt", || ready.exists());
        if operation == "all" {
            let stopped = project
                .build_command()
                .args(["agent", "stop"])
                .env("WAAP_PI_REASONING_EFFORT", "ultra")
                .output()
                .unwrap();
            assert_success(&stopped);
            assert!(String::from_utf8_lossy(&stopped.stdout).contains("Stop accepted for agent"));
        } else {
            assert!(Command::new("kill")
                .args([operation, &runner.id().to_string()])
                .status()
                .unwrap()
                .success());
        }
        assert_success(&wait_output(runner));
        assert_record(&project, &agent_id, "aborted", "pi", "signal-session");
        let commands = fs::read_to_string(log).unwrap();
        assert_eq!(commands.matches("\"type\":\"clear_queue\"").count(), 1);
        assert_eq!(commands.matches("\"type\":\"abort\"").count(), 1);
        assert!(!project.get_socket_path(&agent_id).exists());
        assert!(!project.get_worktree_path(&agent_id).exists());
        assert_reaped(&pid);
    }
}
