#![cfg(target_os = "linux")]
mod common;

use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::UnixListener;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::thread;
use std::time::Duration;

use common::{git, init_repo, isolate_git_config};
use serde_json::{json, Value};

struct Fixture {
    repo: tempfile::TempDir,
    runtime: tempfile::TempDir,
}

impl Fixture {
    fn new() -> Self {
        let fixture = Self {
            repo: tempfile::tempdir().unwrap(),
            runtime: tempfile::tempdir().unwrap(),
        };
        init_repo(fixture.repo.path());
        fs::set_permissions(fixture.runtime.path(), fs::Permissions::from_mode(0o700)).unwrap();
        for args in [vec!["init"], vec!["agent", "new", "--name", "Test"]] {
            let output = fixture.command().args(args).output().unwrap();
            assert!(output.status.success(), "{output:?}");
        }
        fixture.record("running", Some("codex"));
        fixture
    }

    fn command(&self) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_waap"));
        isolate_git_config(&mut command);
        command
            .current_dir(self.repo.path())
            .env("XDG_RUNTIME_DIR", self.runtime.path())
            .args([
                "--waap-root",
                self.repo.path().join(".state").to_str().unwrap(),
            ]);
        command
    }

    fn record(&self, status: &str, system: Option<&str>) {
        fs::write(self.record_path(), format!(
            "+++\ncreation_date = 2026-06-18T15:00:34Z\nstatus = \"{status}\"\nsession_id = \"original-session\"\n{}+++\n\nOriginal instructions.\n",
            system.map(|s| format!("system = \"{s}\"\n")).unwrap_or_default())).unwrap();
        let state = self.repo.path().join(".state");
        git(&state, &["add", "agents"]);
        git(&state, &["commit", "--allow-empty", "-qm", "seed status"]);
    }

    fn record_path(&self) -> PathBuf {
        self.repo.path().join(".state/agents/aa-test/agent.md")
    }

    fn socket(&self) -> PathBuf {
        self.runtime
            .path()
            .join("waap/control")
            .join(
                self.repo
                    .path()
                    .canonicalize()
                    .unwrap()
                    .strip_prefix("/")
                    .unwrap(),
            )
            .join("agents/aa-test.sock")
    }

    fn listener(&self) -> UnixListener {
        let path = self.socket();
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        UnixListener::bind(path).unwrap()
    }

    fn steer(&self, id: &str, prompt: &str) -> Command {
        let mut command = self.command();
        command.args(["agent", "steer", "--agent-id", id, "--prompt", prompt]);
        command
    }

    fn snapshot(&self) -> (Vec<u8>, String, String, String) {
        let state = self.repo.path().join(".state");
        (
            fs::read(self.record_path()).unwrap(),
            git(&state, &["rev-parse", "HEAD"]),
            git(&state, &["status", "--porcelain"]),
            git(self.repo.path(), &["worktree", "list", "--porcelain"]),
        )
    }
}

fn diagnostic(output: &Output) -> String {
    assert!(!output.status.success(), "{output:?}");
    assert!(output.stdout.is_empty(), "{output:?}");
    String::from_utf8(output.stderr.clone()).unwrap()
}

fn serve(listener: &UnixListener, reply: impl FnOnce(&Value) -> Vec<u8>) -> Value {
    let (mut stream, _) = listener.accept().unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    let mut line = String::new();
    BufReader::new(stream.try_clone().unwrap())
        .read_line(&mut line)
        .unwrap();
    let request: Value = serde_json::from_str(&line).unwrap();
    stream.write_all(&reply(&request)).unwrap();
    request
}

fn frame(value: Value) -> Vec<u8> {
    let mut bytes = serde_json::to_vec(&value).unwrap();
    bytes.push(b'\n');
    bytes
}

#[test]
fn cli_preserves_prompt_and_state_with_human_and_json_success() {
    let fixture = Fixture::new();
    let listener = fixture.listener();
    let before = fixture.snapshot();
    let prompt = "  雪\n\"quotes\" 'apostrophe' $(touch injected) ; & `pwd`  ";
    for format in ["human-readable", "json"] {
        thread::scope(|scope| {
            let server = scope.spawn(|| {
                serve(&listener, |request| {
                    frame(json!({
                        "version":1, "request_id":request["request_id"], "accepted":true
                    }))
                })
            });
            let output = fixture
                .steer("aa-test", prompt)
                .args(["--output-format", format])
                .output()
                .unwrap();
            assert!(output.status.success(), "{output:?}");
            assert!(output.stderr.is_empty(), "{output:?}");
            if format == "json" {
                assert_eq!(
                    serde_json::from_slice::<Value>(&output.stdout).unwrap(),
                    json!({"agent_id":"aa-test", "accepted":true})
                );
            } else {
                let stdout = String::from_utf8(output.stdout).unwrap();
                assert!(stdout.contains("aa-test") && stdout.contains("accepted"));
            }
            let request = server.join().unwrap();
            assert_eq!(request["prompt"], prompt);
            assert_eq!(request["version"], 1);
            assert_eq!(request["method"], "steer");
            uuid::Uuid::parse_str(request["request_id"].as_str().unwrap()).unwrap();
        });
        assert_eq!(fixture.snapshot(), before);
        assert!(!fixture.repo.path().join("injected").exists());
    }
}

#[test]
fn validation_rejects_unknown_agents_nonrunning_statuses_and_blank_prompts_before_connect() {
    let fixture = Fixture::new();
    let listener = fixture.listener();
    listener.set_nonblocking(true).unwrap();
    let before = fixture.snapshot();
    for prompt in ["", " \t\r\n", "\u{2003}"] {
        let output = fixture
            .steer("aa-test", prompt)
            .env_remove("XDG_RUNTIME_DIR")
            .output()
            .unwrap();
        assert!(diagnostic(&output).contains("prompt must contain a non-whitespace character"));
    }
    assert!(diagnostic(
        &fixture
            .steer("aa-missing", "data")
            .env_remove("XDG_RUNTIME_DIR")
            .output()
            .unwrap()
    )
    .contains("does not exist"));
    assert_eq!(fixture.snapshot(), before);
    for status in ["ready", "completed", "aborted", "failed"] {
        fixture.record(status, Some("codex"));
        let before = fixture.snapshot();
        let output = fixture
            .steer("aa-test", "data")
            .env_remove("XDG_RUNTIME_DIR")
            .output()
            .unwrap();
        assert!(diagnostic(&output).contains(&format!("is {status}, expected running")));
        assert_eq!(fixture.snapshot(), before);
    }
    assert_eq!(
        listener.accept().unwrap_err().kind(),
        std::io::ErrorKind::WouldBlock
    );
}

#[test]
fn run_preserves_status_validation_and_ready_failure_with_missing_runtime() {
    let fixture = Fixture::new();
    for (agent_id, message) in [
        ("aa-missing", "does not exist"),
        ("aa-test", "running -> running"),
    ] {
        let before = fixture.snapshot();
        let output = fixture
            .command()
            .args(["agent", "run", "--agent-id", agent_id, "--system", "claude"])
            .env_remove("XDG_RUNTIME_DIR")
            .output()
            .unwrap();
        assert!(diagnostic(&output).contains(message));
        assert_eq!(fixture.snapshot(), before);
    }
    fixture.record("ready", None);
    let state = fixture.repo.path().join(".state");
    let before = git(&state, &["rev-parse", "HEAD"]);
    let worktree_paths = || {
        git(fixture.repo.path(), &["worktree", "list", "--porcelain"])
            .lines()
            .filter(|line| line.starts_with("worktree "))
            .map(str::to_owned)
            .collect::<Vec<_>>()
    };
    let worktrees = worktree_paths();
    let output = fixture
        .command()
        .args([
            "agent",
            "run",
            "--agent-id",
            "aa-test",
            "--system",
            "claude",
        ])
        .env_remove("XDG_RUNTIME_DIR")
        .output()
        .unwrap();
    assert!(!output.status.success(), "{output:?}");
    assert!(String::from_utf8_lossy(&output.stderr).contains("XDG_RUNTIME_DIR must be set"));
    assert!(fs::read_to_string(fixture.record_path())
        .unwrap()
        .contains("status = \"failed\""));
    assert_eq!(
        git(&state, &["rev-list", "--count", &format!("{before}..HEAD")]),
        "2"
    );
    assert_eq!(worktree_paths(), worktrees);
    assert!(!fixture.runtime.path().join("waap").exists());
}

#[test]
fn cli_surfaces_backend_rejections_and_uncertainty_without_backend_filtering_or_state_changes() {
    let fixture = Fixture::new();
    let listener = fixture.listener();
    for system in [Some("claude"), Some("opencode"), None] {
        fixture.record("running", system);
        let before = fixture.snapshot();
        let message = format!(
            "steering not implemented for backend {}",
            system.unwrap_or("opencode")
        );
        thread::scope(|scope| {
            let server = scope.spawn(|| {
                serve(&listener, |request| {
                    frame(json!({
                        "version":1,"request_id":request["request_id"],"accepted":false,
                        "error":{"code":"not_implemented","message":message}
                    }))
                })
            });
            assert!(
                diagnostic(&fixture.steer("aa-test", "data").output().unwrap()).contains(&message)
            );
            assert_eq!(server.join().unwrap()["prompt"], "data");
        });
        assert_eq!(fixture.snapshot(), before);
    }
    for mode in ["busy", "acceptance_unknown", "mismatch", "malformed", "eof"] {
        let before = fixture.snapshot();
        thread::scope(|scope| {
            let server = scope.spawn(|| serve(&listener, |request| {
                if mode == "malformed" { return b"not JSON\n".to_vec(); }
                if mode == "eof" { return vec![]; }
                frame(json!({"version":1,
                    "request_id": if mode == "mismatch" {json!(uuid::Uuid::new_v4().to_string())} else {request["request_id"].clone()},
                    "accepted": if mode == "acceptance_unknown" {Value::Null} else {json!(false)},
                    "error":{"code":mode,"message":"Control diagnostic."}}))
            }));
            let text = diagnostic(&fixture.steer("aa-test", "data").output().unwrap());
            assert!(
                text.contains(if mode == "busy" {
                    "busy: Control diagnostic."
                } else {
                    "acceptance is unknown"
                }),
                "{text}"
            );
            server.join().unwrap();
        });
        assert_eq!(fixture.snapshot(), before);
    }
    listener.set_nonblocking(true).unwrap();
    assert_eq!(
        listener.accept().unwrap_err().kind(),
        std::io::ErrorKind::WouldBlock
    );
}

#[test]
fn cli_reports_runtime_configuration_missing_and_stale_socket_errors() {
    let fixture = Fixture::new();
    let before = fixture.snapshot();
    for runtime in [None, Some("")] {
        let mut command = fixture.steer("aa-test", "data");
        match runtime {
            None => {
                command.env_remove("XDG_RUNTIME_DIR");
            }
            Some(value) => {
                command.env("XDG_RUNTIME_DIR", value);
            }
        }
        assert!(diagnostic(&command.output().unwrap()).contains("XDG_RUNTIME_DIR must be set"));
    }
    fs::set_permissions(fixture.runtime.path(), fs::Permissions::from_mode(0o755)).unwrap();
    assert!(
        diagnostic(&fixture.steer("aa-test", "data").output().unwrap()).contains("private (0700)")
    );
    fs::set_permissions(fixture.runtime.path(), fs::Permissions::from_mode(0o700)).unwrap();
    assert!(
        diagnostic(&fixture.steer("aa-test", "data").output().unwrap())
            .contains("cannot connect to control socket")
    );
    drop(fixture.listener());
    assert!(
        diagnostic(&fixture.steer("aa-test", "data").output().unwrap()).contains("stale endpoint")
    );
    assert!(fixture.socket().exists());
    let long = fixture.runtime.path().join("x".repeat(80));
    fs::create_dir(&long).unwrap();
    fs::set_permissions(&long, fs::Permissions::from_mode(0o700)).unwrap();
    assert!(diagnostic(
        &fixture
            .steer("aa-test", "data")
            .env("XDG_RUNTIME_DIR", &long)
            .output()
            .unwrap()
    )
    .contains("platform limit"));
    assert_eq!(fixture.snapshot(), before);
}

#[test]
fn cli_resolves_primary_repository_socket_from_linked_worktree() {
    let fixture = Fixture::new();
    let linked = fixture.repo.path().join("linked");
    git(
        fixture.repo.path(),
        &["worktree", "add", linked.to_str().unwrap(), "-b", "linked"],
    );
    let listener = fixture.listener();
    let before = fixture.snapshot();
    thread::scope(|scope| {
        let server = scope.spawn(|| {
            serve(&listener, |request| {
                frame(json!({"version":1,
            "request_id":request["request_id"],"accepted":true}))
            })
        });
        let output = fixture
            .steer("aa-test", "data")
            .current_dir(Path::new(&linked))
            .output()
            .unwrap();
        assert!(output.status.success(), "{output:?}");
        server.join().unwrap();
    });
    assert_eq!(fixture.snapshot(), before);
}
