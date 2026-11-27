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

const TASK: &str = "  # Test 雪\n\n'quoted' `$HOME`\n";

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
        self.register_agent(name, None)
    }

    fn register_agent(&self, name: &str, system: Option<&str>) -> String {
        let mut args = vec!["--output-format", "json", "agent", "new", "--name", name];
        if let Some(system) = system {
            args.extend(["--system", system]);
        }
        let output = self.run_command("", &args);
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
fn registration_does_not_wait_for_eof_or_start_a_backend() {
    let project = Project::new();
    let record = project.get_agent_record_path("aa-no-input");
    let mut child = project
        .build_command()
        .args([
            "agent",
            "new",
            "--name",
            "No Input",
            "--system",
            "opencode",
            "--output-format",
            "json",
        ])
        .env_remove("OPENCODE_SERVER_URL")
        .env_remove("OPENCODE_SERVER_USERNAME")
        .env_remove("OPENCODE_SERVER_PASSWORD")
        .env_remove("OPENCODE_SERVER_MODEL")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let held_open = child.stdin.take().unwrap();
    let output = wait_output(child);
    drop(held_open);
    assert_success(&output);
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["agent_id"], "aa-no-input");
    assert_eq!(report["metadata"]["status"], "ready");
    assert_eq!(report["metadata"]["system"], "opencode");
    assert!(report["metadata"]["session_id"].is_null());
    let record = fs::read_to_string(record).unwrap();
    assert!(record.ends_with("system = \"opencode\"\n+++\n"));
    assert!(!record.contains("session_id"));
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

    let mut child = project
        .build_command()
        .args([
            "agent",
            "prompt",
            "--agent-id",
            &agent_id,
            "--output-format",
            "json",
        ])
        .env("WAAP_PI_BIN", get_fake_pi_runner_path())
        .env("WAAP_FAKE_PI_SCRIPT", &fake)
        .env("WAAP_FAKE_PID", &pid)
        .env("WAAP_FAKE_RECORD", project.get_agent_record_path(&agent_id))
        .env("WAAP_FAKE_SNAPSHOT", &snapshot)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(TASK.as_bytes())
        .unwrap();
    let output = wait_output(child);
    assert_success(&output);
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("fake-pi-output"));
    assert!(!stdout.contains("delivery") && !stdout.contains("asap"));
    assert_record(&project, &agent_id, "completed", "pi", "pi-process-session");
    let prompt_state = fs::read_to_string(snapshot).unwrap();
    assert!(prompt_state.contains("status = \"running\""));
    assert!(prompt_state.contains("system = \"pi\""));
    assert!(prompt_state.contains("session_id = \"pi-process-session\""));
    let (record, request) = prompt_state.rsplit_once("+++\n").unwrap();
    assert!(!record.contains("# Test"));
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(request).unwrap()["message"],
        TASK
    );
    assert_reaped(&pid);
}

const SAVED_PI_ID: &str = "0193fd5b-d717-7000-8000-123456789abc";

struct ResumeFixture {
    project: Project,
    agent_id: String,
    store: PathBuf,
    session: PathBuf,
    other: PathBuf,
}

impl ResumeFixture {
    fn new(prior: &str, id: &str) -> Self {
        let project = Project::new();
        let agent_id = project.create_agent("resume");
        let record = project.get_agent_record_path(&agent_id);
        let ready = fs::read_to_string(&record).unwrap();
        fs::write(
            record,
            ready.replace(
                "status = \"ready\"",
                &format!("status = \"{prior}\"\nsession_id = \"{id}\""),
            ),
        )
        .unwrap();
        let store = project._temp.path().join("Pi session store");
        fs::create_dir(&store).unwrap();
        let session = store.join("existing.jsonl");
        let other = project._temp.path().join("other.jsonl");
        Self {
            project,
            agent_id,
            store,
            session,
            other,
        }
    }

    fn write_session(&self, path: &Path, id: &str, cwd: &Path) {
        let header = serde_json::json!({"type":"session","version":3,"id":id,"cwd":cwd});
        fs::write(path, format!("{header}\n{{\"type\":\"message\",\"message\":{{\"role\":\"user\",\"content\":\"prior turn\"}}}}\n")).unwrap();
    }

    fn run(&self, mode: &str, store_via: &str) -> Output {
        let subdir = self.project.root.join("nested");
        fs::create_dir(&subdir).unwrap();
        let mut command = self.project.build_command();
        command
            .current_dir(subdir)
            .env("WAAP_PI_BIN", get_fake_pi_runner_path())
            .env("WAAP_PI_MODEL", "env-model")
            .env("WAAP_PI_REASONING_EFFORT", "low")
            .env("WAAP_FAKE_RESUME_MODE", mode)
            .env("WAAP_FAKE_RESUME_ID", SAVED_PI_ID)
            .env("WAAP_FAKE_SESSION_FILE", &self.session)
            .env("WAAP_FAKE_OTHER_FILE", &self.other)
            .env("WAAP_FAKE_PID", self.artifact("pid"))
            .env("WAAP_FAKE_ARGV", self.artifact("argv"))
            .env("WAAP_FAKE_VERSION", self.artifact("version"))
            .env("WAAP_FAKE_CWD", self.artifact("cwd"))
            .env("WAAP_FAKE_LOG", self.artifact("rpc"))
            .env_remove("PI_CODING_AGENT_SESSION_DIR")
            .env("PI_CODING_AGENT_DIR", self.artifact("pi-config"))
            .args([
                "agent",
                "prompt",
                "--delivery=start",
                "--agent-id",
                &self.agent_id,
                "--model",
                "cli-model",
                "--reasoning-effort",
                "off",
                "--text",
                TASK,
            ])
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        match store_via {
            "env" => {
                command.env("PI_CODING_AGENT_SESSION_DIR", "../Pi session store");
            }
            "project-settings" => {
                fs::create_dir(self.project.root.join(".pi")).unwrap();
                fs::write(
                    self.project.root.join(".pi/settings.json"),
                    r#"{"sessionDir":"../Pi session store"}"#,
                )
                .unwrap();
            }
            "global-settings" => {
                fs::create_dir(self.artifact("pi-config")).unwrap();
                fs::write(
                    self.artifact("pi-config/settings.json"),
                    r#"{"sessionDir":"../Pi session store"}"#,
                )
                .unwrap();
            }
            _ => unreachable!(),
        }
        let output = wait_output(command.spawn().unwrap());
        assert!(!self.project.get_socket_path(&self.agent_id).exists());
        output
    }

    fn artifact(&self, name: &str) -> PathBuf {
        self.project._temp.path().join(name)
    }
}

#[test]
fn test_resume_opens_exact_existing_history_and_submits_once() {
    for (prior, store_via) in [
        ("completed", "env"),
        ("aborted", "project-settings"),
        ("completed", "global-settings"),
    ] {
        let fixture = ResumeFixture::new(prior, SAVED_PI_ID);
        fixture.write_session(&fixture.session, SAVED_PI_ID, &fixture.project.root);
        assert_success(&fixture.run("exact", store_via));
        assert_record(
            &fixture.project,
            &fixture.agent_id,
            "completed",
            "pi",
            SAVED_PI_ID,
        );
        assert_eq!(
            fs::read_to_string(fixture.artifact("cwd"))
                .unwrap()
                .trim_end(),
            fixture.project.root.to_str().unwrap()
        );
        let argv = fs::read_to_string(fixture.artifact("argv")).unwrap();
        assert_eq!(
            argv.lines().collect::<Vec<_>>(),
            [
                "--mode",
                "rpc",
                "--approve",
                "--session",
                SAVED_PI_ID,
                "--session-dir",
                fixture.store.to_str().unwrap(),
                "--model",
                "cli-model",
                "--thinking",
                "off",
            ]
        );
        let commands = fs::read_to_string(fixture.artifact("rpc")).unwrap();
        let commands: Vec<serde_json::Value> = commands
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect();
        assert_eq!(commands.len(), 2);
        assert_eq!(commands[0]["type"], "get_state");
        assert_eq!(commands[1]["type"], "prompt");
        assert_eq!(commands[1]["message"], TASK);
        assert_reaped(&fixture.artifact("pid"));
    }
}

#[test]
fn test_resume_preflight_failures_do_not_launch_pi() {
    for (prior, mode, error) in [
        ("completed", "missing", "not found"),
        ("aborted", "prefix-only", "not found"),
        ("completed", "partial-id", "full Pi session UUID"),
        (
            "completed",
            "wrong-checkout",
            "run from the original checkout",
        ),
        ("aborted", "duplicate", "multiple Pi files match"),
        (
            "completed",
            "unsupported",
            "requires verified Pi version 1.5.2",
        ),
    ] {
        let id = if mode == "partial-id" {
            &SAVED_PI_ID[..8]
        } else {
            SAVED_PI_ID
        };
        let fixture = ResumeFixture::new(prior, id);
        let root = &fixture.project.root;
        match mode {
            "prefix-only" => fixture.write_session(&fixture.session, &format!("{id}-other"), root),
            "partial-id" | "unsupported" => {
                fixture.write_session(&fixture.session, SAVED_PI_ID, root)
            }
            "wrong-checkout" => {
                fixture.write_session(&fixture.session, id, fixture.project._temp.path())
            }
            "duplicate" => {
                fixture.write_session(&fixture.session, id, root);
                fixture.write_session(&fixture.store.join("duplicate.jsonl"), id, root);
            }
            _ => (),
        }
        let record = fixture.project.get_agent_record_path(&fixture.agent_id);
        let before = fs::read(&record).unwrap();
        let output = fixture.run(mode, "env");
        assert!(!output.status.success());
        assert!(
            String::from_utf8_lossy(&output.stderr).contains(error),
            "{output:?}"
        );
        assert_eq!(fs::read(record).unwrap(), before);
        assert_eq!(fixture.artifact("version").exists(), mode == "unsupported");
        assert!(!fixture.artifact("pid").exists());
        assert!(!fixture.artifact("rpc").exists());
    }
}

#[test]
fn test_resume_rejects_wrong_rpc_identity_or_lost_history_without_submitting() {
    for (prior, mode, error) in [
        ("completed", "wrong-id", "but RPC returned"),
        ("aborted", "wrong-file", "different session file"),
        (
            "completed",
            "changed-checkout",
            "run from the original checkout",
        ),
        (
            "completed",
            "empty-history",
            "did not report reopened conversation history",
        ),
        ("aborted", "deleted", "before submission"),
    ] {
        let fixture = ResumeFixture::new(prior, SAVED_PI_ID);
        fixture.write_session(&fixture.session, SAVED_PI_ID, &fixture.project.root);
        if mode == "wrong-file" {
            fixture.write_session(&fixture.other, "different-header-id", &fixture.project.root);
        } else if mode == "changed-checkout" {
            fixture.write_session(&fixture.other, SAVED_PI_ID, fixture.project._temp.path());
        }
        let record = fixture.project.get_agent_record_path(&fixture.agent_id);
        let before = fs::read(&record).unwrap();
        let session_bytes = fs::read(&fixture.session).unwrap();
        let output = fixture.run(mode, "env");
        assert!(!output.status.success());
        assert!(
            String::from_utf8_lossy(&output.stderr).contains(error),
            "{output:?}"
        );
        assert_eq!(fs::read(record).unwrap(), before);
        if mode == "deleted" {
            assert_eq!(fs::read_dir(&fixture.store).unwrap().count(), 0);
        } else {
            let expected_bytes = if mode == "changed-checkout" {
                fs::read(&fixture.other).unwrap()
            } else {
                session_bytes
            };
            assert_eq!(fs::read(&fixture.session).unwrap(), expected_bytes);
            let rpc = fs::read_to_string(fixture.artifact("rpc")).unwrap();
            let command: serde_json::Value = serde_json::from_str(&rpc).unwrap();
            assert_eq!(command["type"], "get_state");
        }
        assert_eq!(
            fs::read_to_string(fixture.artifact("argv"))
                .unwrap()
                .matches("--session\n")
                .count(),
            1
        );
        assert_reaped(&fixture.artifact("pid"));
    }
}

#[test]
fn test_fake_pi_failure_exits_nonzero_persists_failed() {
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
        .args([
            "agent",
            "prompt",
            "--delivery=start",
            "--agent-id",
            &agent_id,
            "--text",
            TASK,
        ])
        .env("WAAP_PI_BIN", get_fake_pi_runner_path())
        .env("WAAP_FAKE_PI_SCRIPT", fake)
        .env("WAAP_FAKE_PID", &pid)
        .output()
        .unwrap();

    assert!(!output.status.success());
    assert_record(&project, &agent_id, "failed", "pi", "pi-failed-session");
    assert_reaped(&pid);
}

#[test]
fn test_stop_uses_shared_socket_after_steering_and_owner_persists_aborted() {
    assert_pi_stop_outcome("aborted", "aborted");
}

#[test]
fn test_pi_natural_completion_wins_race_with_acknowledged_stop() {
    assert_pi_stop_outcome("stop", "completed");
}

fn assert_pi_stop_outcome(stop_reason: &str, status: &str) {
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
      printf '%s\n' '{"type":"message_end","message":{"role":"assistant","stopReason":"STOP_REASON"}}'
      printf '%s\n' '{"type":"agent_settled"}'
      printf '%s\n' '{"type":"response","id":"waap-5","command":"abort","success":true}'
      ;;
  esac
done
"#.replace("STOP_REASON", stop_reason) + &build_shutdown_notification_script()),
    );
    let runner = project
        .build_command()
        .args([
            "agent",
            "prompt",
            "--delivery=start",
            "--agent-id",
            &agent_id,
            "--text",
            TASK,
        ])
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
            "prompt",
            "--delivery=steer",
            "--agent-id",
            &agent_id,
            "--text",
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
    assert_record(&project, &agent_id, status, "pi", "pi-abort-session");
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
        ready
            .replace(
                "status = \"ready\"",
                "status = \"running\"\nsession_id = \"legacy-session\"",
            )
            .replace("system = \"pi\"\n", ""),
    )
    .unwrap();
    let before_record = fs::read(&record_path).unwrap();
    let before_head = git(&project.state, &["rev-parse", "HEAD"]);

    let output = project.run_command("", &["agent", "stop", "--agent-id", &agent_id]);

    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("field system"));
    assert_eq!(fs::read(record_path).unwrap(), before_record);
    assert_eq!(git(&project.state, &["rev-parse", "HEAD"]), before_head);
}

#[test]
fn test_codex_shared_control_stop_keeps_owner_outcome_and_cleanup() {
    let project = Project::new();
    let agent_id = project.register_agent("codex-process-abort", Some("codex"));
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
        .args([
            "agent",
            "prompt",
            "--delivery=start",
            "--agent-id",
            &agent_id,
            "--text",
            TASK,
        ])
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
                .args([
                    "agent",
                    "prompt",
                    "--delivery=start",
                    "--agent-id",
                    &agent_id,
                    "--text",
                    TASK,
                ])
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
                .args([
                    "agent",
                    "prompt",
                    "--delivery=start",
                    "--agent-id",
                    &agent_id,
                    "--text",
                    TASK,
                ])
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
                .args([
                    "agent",
                    "prompt",
                    "--delivery=start",
                    "--agent-id",
                    &agent_id,
                    "--text",
                    TASK,
                ])
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
        .args([
            "agent",
            "prompt",
            "--delivery=start",
            "--agent-id",
            &agent_id,
            "--text",
            TASK,
        ])
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
        let output = wait_output(project.build_command().args(["agent", "prompt", "--delivery=start", "--agent-id", &agent_id, "--text", TASK])
            .env("WAAP_PI_BIN", get_fake_pi_runner_path()).env("WAAP_FAKE_PI_SCRIPT", fake).env("WAAP_FAKE_PID", &pid)
            .stdout(Stdio::piped()).stderr(Stdio::piped()).spawn().unwrap());
        assert!(!output.status.success());
        let record = fs::read_to_string(project.get_agent_record_path(&agent_id)).unwrap();
        assert!(record.contains("status = \"failed\""));
        assert!(!record.contains("session_id"));
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
        .args([
            "agent",
            "prompt",
            "--delivery=start",
            "--agent-id",
            &agent_id,
            "--text",
            TASK,
        ])
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
            .args([
                "agent",
                "prompt",
                "--delivery=start",
                "--agent-id",
                &agent_id,
                "--text",
                TASK,
            ])
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
        assert_reaped(&pid);
    }
}
