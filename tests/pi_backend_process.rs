//! Process-level coverage for the Pi default and direct-owner abort lifecycle.

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
        let state = root.join(".state");
        let project = Self {
            _temp: temp,
            root,
            state,
        };
        let output = project.output("", &["init"]);
        assert_success(&output);
        project
    }

    fn command(&self) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_waap"));
        isolate_git_config(&mut command);
        command
            .current_dir(&self.root)
            .arg("--waap-root")
            .arg(&self.state);
        command
    }

    fn output(&self, stdin: &str, args: &[&str]) -> Output {
        let mut child = self
            .command()
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
        let output = self.output(
            "Complete the deterministic fixture.\n",
            &["--output-format", "json", "agent", "new", "--name", name],
        );
        assert_success(&output);
        serde_json::from_slice::<serde_json::Value>(&output.stdout).unwrap()["agent_id"]
            .as_str()
            .unwrap()
            .to_string()
    }

    fn agent_record(&self, agent_id: &str) -> PathBuf {
        self.state.join("agents").join(agent_id).join("agent.md")
    }

    fn worktree(&self, agent_id: &str) -> PathBuf {
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

fn wait_output(child: Child) -> Output {
    child.wait_with_output().unwrap()
}

fn assert_record(project: &Project, agent_id: &str, status: &str, system: &str, session: &str) {
    let record = fs::read_to_string(project.agent_record(agent_id)).unwrap();
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
fn omitted_system_runs_fake_pi_after_authentic_session_is_persisted() {
    let project = Project::new();
    let agent_id = project.create_agent("pi-process-success");
    let fake = project.root.join("fake-pi-success");
    let snapshot = project.root.join("prompt-state");
    write_executable(
        &fake,
        r#"#!/bin/sh
set -eu
IFS= read -r state
printf '%s\n' '{"type":"response","id":"waap-1","command":"get_state","success":true,"data":{"sessionId":"pi-process-session"}}'
IFS= read -r prompt
cp "$WAAP_FAKE_RECORD" "$WAAP_FAKE_SNAPSHOT"
printf '%s\n' "$prompt" >> "$WAAP_FAKE_SNAPSHOT"
printf '%s\n' '{"type":"response","id":"waap-2","command":"prompt","success":true}'
printf '%s\n' '{"type":"message_update","assistantMessageEvent":{"type":"text_delta","delta":"fake-pi-output"}}'
printf '%s\n' '{"type":"message_end","message":{"role":"assistant","stopReason":"stop"}}'
printf '%s\n' '{"type":"agent_settled"}'
while IFS= read -r ignored; do :; done
"#,
    );

    let output = project
        .command()
        .args(["agent", "run", "--agent-id", &agent_id])
        .env("WAAP_PI_BIN", &fake)
        .env("WAAP_FAKE_RECORD", project.agent_record(&agent_id))
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
    assert!(!project.worktree(&agent_id).exists());
    let subjects = git(&project.state, &["log", "--pretty=%s"]);
    assert!(subjects.contains(&format!("waap agent run {agent_id}")));
    assert!(subjects.contains(&format!("waap agent pi session {agent_id}")));
    assert!(subjects.contains(&format!("waap agent completed {agent_id}")));
}

#[test]
fn fake_pi_failure_exits_nonzero_persists_failed_and_cleans_worktree() {
    let project = Project::new();
    let agent_id = project.create_agent("pi-process-failure");
    let fake = project.root.join("fake-pi-failure");
    write_executable(
        &fake,
        r#"#!/bin/sh
set -eu
IFS= read -r state
printf '%s\n' '{"type":"response","id":"waap-1","command":"get_state","success":true,"data":{"sessionId":"pi-failed-session"}}'
IFS= read -r prompt
printf '%s\n' '{"type":"response","id":"waap-2","command":"prompt","success":true}'
printf '%s\n' '{"type":"message_end","message":{"role":"assistant","stopReason":"error"}}'
printf '%s\n' '{"type":"agent_settled"}'
while IFS= read -r ignored; do :; done
"#,
    );

    let output = project
        .command()
        .args(["agent", "run", "--agent-id", &agent_id, "--system", "pi"])
        .env("WAAP_PI_BIN", fake)
        .output()
        .unwrap();

    assert!(!output.status.success());
    assert_record(&project, &agent_id, "failed", "pi", "pi-failed-session");
    assert!(!project.worktree(&agent_id).exists());
}

#[test]
fn stop_signals_pi_owner_sends_rpc_abort_and_both_converge() {
    let project = Project::new();
    let agent_id = project.create_agent("pi-process-abort");
    let fake = project.root.join("fake-pi-abort");
    let log = project.root.join("pi-commands");
    let ready = project.root.join("pi-ready");
    write_executable(
        &fake,
        r#"#!/bin/sh
set -eu
while IFS= read -r line; do
  printf '%s\n' "$line" >> "$WAAP_FAKE_LOG"
  case "$line" in
    *'"type":"get_state"'*)
      printf '%s\n' '{"type":"response","id":"waap-1","command":"get_state","success":true,"data":{"sessionId":"pi-abort-session"}}'
      ;;
    *'"type":"prompt"'*)
      printf '%s\n' '{"type":"response","id":"waap-2","command":"prompt","success":true}'
      touch "$WAAP_FAKE_READY"
      ;;
    *'"type":"abort"'*)
      printf '%s\n' '{"type":"response","id":"waap-3","command":"abort","success":true}'
      printf '%s\n' '{"type":"message_end","message":{"role":"assistant","stopReason":"aborted"}}'
      printf '%s\n' '{"type":"agent_settled"}'
      ;;
  esac
done
"#,
    );
    let runner = project
        .command()
        .args(["agent", "run", "--agent-id", &agent_id, "--system", "pi"])
        .env("WAAP_PI_BIN", fake)
        .env("WAAP_FAKE_LOG", &log)
        .env("WAAP_FAKE_READY", &ready)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    wait_for("Pi prompt acceptance", || ready.exists());

    let stop = project.output("", &["agent", "stop", "--agent-id", &agent_id]);
    assert_success(&stop);
    let owner = wait_output(runner);

    assert!(!owner.status.success());
    assert!(!String::from_utf8_lossy(&owner.stderr).contains("transition"));
    assert_record(&project, &agent_id, "aborted", "pi", "pi-abort-session");
    assert_eq!(
        fs::read_to_string(log)
            .unwrap()
            .matches("\"type\":\"abort\"")
            .count(),
        1
    );
    assert!(!project.worktree(&agent_id).exists());
}

#[test]
fn stop_rejects_running_record_without_system_without_mutation() {
    let project = Project::new();
    let agent_id = project.create_agent("legacy-sessionless-system");
    let record_path = project.agent_record(&agent_id);
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

    let output = project.output("", &["agent", "stop", "--agent-id", &agent_id]);

    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("no persisted system"));
    assert_eq!(fs::read(record_path).unwrap(), before_record);
    assert_eq!(git(&project.state, &["rev-parse", "HEAD"]), before_head);
}

#[test]
fn stop_signals_codex_owner_and_both_converge_without_transition_errors() {
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
        .command()
        .args(["agent", "run", "--agent-id", &agent_id, "--system", "codex"])
        .env("PATH", path)
        .env("WAAP_FAKE_LOG", &log)
        .env("WAAP_FAKE_READY", &ready)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    wait_for("Codex turn start", || ready.exists());

    let stop = project.output("", &["agent", "stop", "--agent-id", &agent_id]);
    assert_success(&stop);
    let owner = wait_output(runner);

    assert!(!owner.status.success());
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
    assert!(!project.worktree(&agent_id).exists());
}
