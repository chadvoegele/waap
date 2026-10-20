use std::io::{BufRead, BufReader};
use std::sync::Mutex;

use serde_json::json;
use tempfile::TempDir;
use uuid::Uuid;

use super::*;
use crate::agent::backend::{fake::FakeBackend, AgentSystemBackend, StartContext};
use crate::test_git::init_repo_with_commit;

struct Fixture {
    server: ControlServer,
    runtime: TempDir,
    repo: TempDir,
    commands: SyncSender<(String, Instant)>,
    command_rx: Receiver<(String, Instant)>,
    replies: mpsc::Sender<DispatchResult>,
    reply_rx: Receiver<DispatchResult>,
}

impl Fixture {
    fn new() -> Self {
        let runtime = tempfile::tempdir().unwrap();
        fs::set_permissions(runtime.path(), fs::Permissions::from_mode(0o700)).unwrap();
        let repo = tempfile::tempdir().unwrap();
        init_repo_with_commit(repo.path());
        let server =
            ControlServer::bind(repo.path(), "aa-1", Some(runtime.path().as_os_str())).unwrap();
        let (commands, command_rx) = mpsc::sync_channel(1);
        let (replies, reply_rx) = mpsc::channel();
        Self {
            server,
            runtime,
            repo,
            commands,
            command_rx,
            replies,
            reply_rx,
        }
    }

    fn tick(&mut self) {
        self.server
            .service(&self.commands, &self.reply_rx, Instant::now());
    }

    fn connect(&self, bytes: &[u8]) -> UnixStream {
        let mut client = UnixStream::connect(&self.server.path).unwrap();
        client
            .set_read_timeout(Some(Duration::from_secs(3)))
            .unwrap();
        client.write_all(bytes).unwrap();
        client
    }

    fn request(&self, index: u128, prompt: &str) -> UnixStream {
        self.connect(&frame(index, prompt))
    }

    fn acknowledge(&mut self, result: Result<(), SteeringError>) {
        self.replies
            .send(DispatchResult {
                result,
                completed_at: Instant::now(),
                called_backend: true,
            })
            .unwrap();
        self.tick();
    }
}

fn request(index: u128, prompt: &str) -> Value {
    json!({"version": 1, "request_id": Uuid::from_u128(index).to_string(), "method": "steer", "prompt": prompt})
}

fn frame(index: u128, prompt: &str) -> Vec<u8> {
    let mut bytes = serde_json::to_vec(&request(index, prompt)).unwrap();
    bytes.push(b'\n');
    bytes
}

fn read_response(client: UnixStream) -> Value {
    let mut reader = BufReader::new(client);
    let mut line = String::new();
    reader.read_line(&mut line).unwrap();
    assert!(line.ends_with('\n'), "expected a complete response: {line}");
    let result = serde_json::from_str(&line).unwrap();
    line.clear();
    match reader.read_line(&mut line) {
        Ok(0) => {}
        Err(error) if error.kind() == io::ErrorKind::ConnectionReset => {}
        result => panic!("expected connection closure after one response: {result:?}"),
    }
    result
}

fn assert_error(value: &Value, accepted: Value, code: &str) {
    assert_eq!(value["version"], 1);
    assert_eq!(value["accepted"], accepted);
    assert_eq!(value["error"]["code"], code);
}

#[test]
fn socket_rejects_invalid_schemas_without_backend_dispatch_or_prompt_disclosure() {
    let mut fixture = Fixture::new();
    let secret = "private prompt";
    let valid = request(1, secret);
    let mut cases = vec![
        (b"not JSON\n".to_vec(), "invalid_request", false),
        (b"\xff\n".to_vec(), "invalid_request", false),
        (b"[]\n".to_vec(), "invalid_request", false),
    ];
    for (key, value, code, trusted_id) in [
        ("version", json!(2), "unsupported_version", true),
        ("version", json!("1"), "invalid_request", true),
        ("method", json!("start"), "unknown_method", true),
        ("method", json!(1), "invalid_request", true),
        ("request_id", json!("untrusted"), "invalid_request", false),
        ("request_id", Value::Null, "invalid_request", false),
        ("prompt", json!(" \n\t\u{2003}"), "invalid_request", true),
        ("prompt", json!(7), "invalid_request", true),
        ("threadId", json!("not allowed"), "invalid_request", true),
    ] {
        let mut value_to_send = valid.clone();
        value_to_send[key] = value;
        let mut bytes = serde_json::to_vec(&value_to_send).unwrap();
        bytes.push(b'\n');
        cases.push((bytes, code, trusted_id));
    }
    for key in ["version", "method", "prompt", "request_id"] {
        let mut value = valid.clone();
        value.as_object_mut().unwrap().remove(key);
        let mut bytes = serde_json::to_vec(&value).unwrap();
        bytes.push(b'\n');
        cases.push((bytes, "invalid_request", key != "request_id"));
    }
    for (bytes, code, trusted_id) in cases {
        let client = fixture.connect(&bytes);
        fixture.tick();
        let value = read_response(client);
        assert_error(&value, json!(false), code);
        assert_eq!(
            value["request_id"],
            if trusted_id {
                valid["request_id"].clone()
            } else {
                Value::Null
            }
        );
        assert!(!value.to_string().contains(secret));
        assert!(fixture.command_rx.try_recv().is_err());
    }
}

#[test]
fn real_socket_preserves_prompt_and_correlates_backend_results() {
    for error in [
        None,
        Some(SteeringError::Rejected {
            code: "no_active_turn",
            message: "No active turn.".to_string(),
        }),
        Some(SteeringError::Rejected {
            code: "backend_rejected",
            message: "Rejected.".to_string(),
        }),
        Some(SteeringError::AcceptanceUnknown {
            message: "Steering acceptance is unknown.".to_string(),
        }),
    ] {
        let fixture = Fixture::new();
        let path = fixture.server.path.clone();
        let endpoint = path.clone();
        let (release, wait) = mpsc::channel();
        let mut backend = FakeBackend {
            steer_error: error.clone(),
            wait_action: Some(Box::new(move || {
                wait.recv_timeout(Duration::from_secs(5)).unwrap();
                Ok(())
            })),
            ..FakeBackend::default()
        };
        let prompt = "  steer\nλ, quotes '\"; $() `data`  ";
        let started = backend
            .start(StartContext {
                agent_id: "aa-1",
                prompt: "original",
                repository_root: fixture.repo.path(),
                worktree_dir: fixture.repo.path(),
            })
            .unwrap();
        let client = thread::spawn(move || {
            let mut stream = UnixStream::connect(&path).unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(3)))
                .unwrap();
            let bytes = frame(21, prompt);
            stream.write_all(&bytes[..20]).unwrap();
            stream.write_all(&bytes[20..]).unwrap();
            let result = read_response(stream);
            release.send(()).unwrap();
            result
        });
        assert_eq!(
            fixture.server.wait(started.handle.as_ref()).unwrap(),
            RunOutcome::Completed
        );
        let actual = client.join().unwrap();
        assert_eq!(
            actual,
            response(&Uuid::from_u128(21).to_string(), error.map_or(Ok(()), Err))
        );
        assert_eq!(backend.steer_calls.lock().unwrap().len(), 1);
        assert_eq!(backend.steer_calls.lock().unwrap()[0].0, prompt);
        assert!(!endpoint.exists());
    }
}

#[test]
fn completion_progresses_during_dispatch_and_rejects_queued_requests() {
    struct BlockingRun {
        completion: Mutex<Receiver<()>>,
        entered: mpsc::Sender<String>,
        release: Mutex<Receiver<()>>,
    }
    impl RunHandle for BlockingRun {
        fn wait(&self) -> io::Result<RunOutcome> {
            self.completion
                .lock()
                .unwrap()
                .recv_timeout(Duration::from_secs(5))
                .unwrap();
            Ok(RunOutcome::Completed)
        }

        fn steer(&self, prompt: &str, _deadline: Instant) -> Result<(), SteeringError> {
            self.entered.send(prompt.to_string()).unwrap();
            self.release
                .lock()
                .unwrap()
                .recv_timeout(Duration::from_secs(5))
                .unwrap();
            Ok(())
        }
    }
    let fixture = Fixture::new();
    let path = fixture.server.path.clone();
    let (complete, completion) = mpsc::channel();
    let (entered, calls) = mpsc::channel();
    let (release, blocked) = mpsc::channel();
    let run = BlockingRun {
        completion: Mutex::new(completion),
        entered,
        release: Mutex::new(blocked),
    };
    let first = fixture.request(1, "blocked");
    thread::scope(|scope| {
        let owner = scope.spawn(|| fixture.server.wait(&run));
        assert_eq!(
            calls.recv_timeout(Duration::from_secs(3)).unwrap(),
            "blocked"
        );
        let connect = |index| {
            let mut stream = UnixStream::connect(&path).unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(3)))
                .unwrap();
            stream.write_all(&frame(index, "pending")).unwrap();
            stream
        };
        let pending: Vec<_> = (2..=17).map(connect).collect();
        let overflow = connect(18);
        assert_error(&read_response(overflow), json!(false), "busy");
        complete.send(()).unwrap();
        assert_error(&read_response(first), Value::Null, "acceptance_unknown");
        for client in pending {
            assert_error(&read_response(client), json!(false), "agent_exited");
        }
        assert!(
            !path.exists(),
            "endpoint is removed before joining late dispatch"
        );
        assert!(
            calls.try_recv().is_err(),
            "queued requests did not dispatch"
        );
        release.send(()).unwrap();
        assert_eq!(owner.join().unwrap().unwrap(), RunOutcome::Completed);
    });
}

#[test]
fn fifo_allows_one_dispatch_and_sixteen_pending_then_rejects_overflow() {
    let mut fixture = Fixture::new();
    let first = fixture.request(1, "first");
    fixture.tick();
    let first_call = fixture.command_rx.try_recv().unwrap();
    assert_eq!(first_call.0, "first");
    let mut pending = Vec::new();
    for index in 2..=17 {
        pending.push((index, fixture.request(index, &index.to_string())));
        fixture.tick();
    }
    assert_eq!(fixture.server.pending.len(), MAX_PENDING);
    let overflow = fixture.request(18, "overflow");
    fixture.tick();
    let value = read_response(overflow);
    assert_error(&value, json!(false), "busy");
    assert_eq!(value["request_id"], Uuid::from_u128(18).to_string());
    assert!(fixture.command_rx.try_recv().is_err());
    fixture.acknowledge(Ok(()));
    assert_eq!(
        read_response(first)["request_id"],
        Uuid::from_u128(1).to_string()
    );
    for (index, client) in pending {
        let (prompt, deadline) = fixture.command_rx.try_recv().unwrap();
        assert_eq!(prompt, index.to_string());
        assert!(deadline >= first_call.1);
        fixture.acknowledge(Ok(()));
        assert_eq!(
            read_response(client),
            response(&Uuid::from_u128(index).to_string(), Ok(()))
        );
    }
    assert!(fixture.server.dispatch.is_none());
    assert!(fixture.server.pending.is_empty());
}

#[test]
fn frame_limit_includes_the_newline_and_connection_serves_only_one_request() {
    let mut fixture = Fixture::new();
    let overhead = frame(1, "").len();
    let prompt = "x".repeat(MAX_FRAME_BYTES - overhead);
    let bytes = frame(1, &prompt);
    assert_eq!(bytes.len(), MAX_FRAME_BYTES);
    let client = fixture.connect(&bytes);
    fixture.tick();
    assert_eq!(fixture.command_rx.try_recv().unwrap().0, prompt);
    fixture.acknowledge(Ok(()));
    assert_eq!(read_response(client)["accepted"], true);
    let oversized = fixture.connect(&frame(2, &format!("{prompt}x")));
    fixture.tick();
    assert_error(&read_response(oversized), json!(false), "request_too_large");
    assert!(fixture.command_rx.try_recv().is_err());
    let mut two_frames = frame(3, "one");
    two_frames.extend(frame(4, "two"));
    let client = fixture.connect(&two_frames);
    fixture.tick();
    assert_eq!(fixture.command_rx.try_recv().unwrap().0, "one");
    fixture.acknowledge(Ok(()));
    assert_eq!(
        read_response(client)["request_id"],
        Uuid::from_u128(3).to_string()
    );
    assert!(fixture.command_rx.try_recv().is_err());
}

#[test]
fn connection_cap_and_absolute_incomplete_frame_deadline() {
    let mut fixture = Fixture::new();
    let mut clients: Vec<_> = (0..MAX_CONNECTIONS)
        .map(|_| fixture.connect(b"{"))
        .collect();
    fixture.tick();
    assert_eq!(fixture.server.connections.len(), MAX_CONNECTIONS);
    let mut excess = fixture.connect(b"");
    fixture.tick();
    let mut byte = [0];
    assert_eq!(excess.read(&mut byte).unwrap(), 0);
    assert_eq!(fixture.server.connections.len(), MAX_CONNECTIONS);
    let deadline = fixture
        .server
        .connections
        .values()
        .map(|connection| {
            let Stage::Reading { deadline, .. } = connection.stage else {
                panic!("reading")
            };
            deadline
        })
        .max()
        .unwrap();
    clients[0].write_all(b" ").unwrap();
    fixture
        .server
        .read_requests(deadline - Duration::from_millis(1));
    fixture
        .server
        .flush_replies(deadline - Duration::from_millis(1));
    assert_eq!(fixture.server.connections.len(), MAX_CONNECTIONS);
    fixture.server.read_requests(deadline);
    fixture.server.flush_replies(deadline);
    assert!(fixture.server.connections.is_empty());
    for mut client in clients {
        assert_eq!(client.read(&mut byte).unwrap(), 0);
    }
    assert!(fixture.command_rx.try_recv().is_err());
    assert_eq!(FRAME_TIMEOUT, Duration::from_secs(5));
}

#[test]
fn queued_expiry_is_rejection_and_late_ack_cannot_dispatch_concurrently_or_misroute() {
    let mut fixture = Fixture::new();
    let first = fixture.request(1, "first");
    fixture.tick();
    let (_, dispatch_deadline) = fixture.command_rx.try_recv().unwrap();
    let second = fixture.request(2, "expires queued");
    fixture.tick();
    let queued_deadline = fixture.server.pending.front().unwrap().deadline;
    assert!(queued_deadline >= dispatch_deadline);
    assert_eq!(ACK_TIMEOUT, Duration::from_secs(30));
    let now = queued_deadline;
    fixture.server.expire_requests(now);
    fixture.server.flush_replies(now);
    assert_error(&read_response(first), Value::Null, "acceptance_unknown");
    assert_error(&read_response(second), json!(false), "timeout");
    assert!(fixture.server.pending.is_empty());
    assert!(fixture.server.dispatch.as_ref().unwrap().responded);
    let third = fixture.request(3, "next");
    fixture.tick();
    assert!(
        fixture.command_rx.try_recv().is_err(),
        "late acknowledgement still owns dispatch slot"
    );
    fixture
        .replies
        .send(DispatchResult {
            result: Ok(()),
            completed_at: now,
            called_backend: true,
        })
        .unwrap();
    fixture.tick();
    assert_eq!(fixture.command_rx.try_recv().unwrap().0, "next");
    fixture.acknowledge(Err(SteeringError::Rejected {
        code: "backend_rejected",
        message: "third only".to_string(),
    }));
    let value = read_response(third);
    assert_eq!(value["request_id"], Uuid::from_u128(3).to_string());
    assert_error(&value, json!(false), "backend_rejected");
}

#[test]
fn dispatch_rechecks_deadline_and_never_resets_queued_deadline() {
    let mut fixture = Fixture::new();
    let first = fixture.request(1, "first");
    fixture.tick();
    fixture.command_rx.try_recv().unwrap();
    let second = fixture.request(2, "second");
    fixture.tick();
    let deadline = fixture.server.pending.front().unwrap().deadline;
    fixture.acknowledge(Ok(()));
    read_response(first);
    assert_eq!(fixture.command_rx.try_recv().unwrap().1, deadline);
    fixture.acknowledge(Ok(()));
    read_response(second);

    let third = fixture.request(3, "expired");
    fixture.server.accept_connections(Instant::now());
    fixture.server.read_requests(Instant::now());
    let deadline = fixture.server.pending.front().unwrap().deadline;
    fixture.server.dispatch_next(&fixture.commands, deadline);
    fixture.server.flush_replies(deadline);
    assert_error(&read_response(third), json!(false), "timeout");
    assert!(fixture.command_rx.try_recv().is_err());
}

#[test]
fn acknowledgement_time_determines_acceptance_even_when_owner_polls_after_deadline() {
    for timely in [true, false] {
        let mut fixture = Fixture::new();
        let client = fixture.request(1, "steer");
        fixture.tick();
        fixture.command_rx.try_recv().unwrap();
        let deadline = fixture.server.dispatch.as_ref().unwrap().pending.deadline;
        fixture
            .replies
            .send(DispatchResult {
                result: Ok(()),
                completed_at: if timely {
                    deadline - Duration::from_nanos(1)
                } else {
                    deadline
                },
                called_backend: true,
            })
            .unwrap();
        fixture.server.service(
            &fixture.commands,
            &fixture.reply_rx,
            deadline + Duration::from_millis(1),
        );
        let value = read_response(client);
        if timely {
            assert_eq!(value["accepted"], true);
        } else {
            assert_error(&value, Value::Null, "acceptance_unknown");
        }
    }
}

#[test]
fn same_agent_id_in_two_repositories_uses_independent_sockets_and_dispatch() {
    let mut first = Fixture::new();
    let mut second = Fixture::new();
    second.server = ControlServer::bind(
        second.repo.path(),
        "aa-1",
        Some(first.runtime.path().as_os_str()),
    )
    .unwrap();
    assert_ne!(first.server.path, second.server.path);
    let client_a = first.request(1, "repository a");
    let client_b = second.request(1, "repository b");
    first.tick();
    second.tick();
    assert_eq!(first.command_rx.try_recv().unwrap().0, "repository a");
    assert_eq!(second.command_rx.try_recv().unwrap().0, "repository b");
    second.acknowledge(Err(SteeringError::not_implemented("claude")));
    first.acknowledge(Ok(()));
    assert_eq!(read_response(client_a)["accepted"], true);
    assert_error(&read_response(client_b), json!(false), "not_implemented");
}

#[test]
fn completion_rejects_pending_and_cleans_endpoint_without_repairing_stale_files() {
    let mut fixture = Fixture::new();
    let path = fixture.server.path.clone();
    assert_eq!(
        fs::metadata(&path).unwrap().permissions().mode() & 0o777,
        0o600
    );
    assert_eq!(
        fs::metadata(path.parent().unwrap())
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o700
    );
    let first = fixture.request(1, "dispatched");
    fixture.tick();
    fixture.command_rx.try_recv().unwrap();
    let second = fixture.request(2, "queued");
    fixture.tick();
    fixture.server.finish(Instant::now());
    fixture.server.remove_socket().unwrap();
    assert_error(&read_response(first), Value::Null, "acceptance_unknown");
    assert_error(&read_response(second), json!(false), "agent_exited");
    assert!(!path.exists());
    assert!(UnixStream::connect(&path).is_err());
    let stale = UnixListener::bind(&path).unwrap();
    drop(stale);
    assert!(UnixStream::connect(&path).is_err());
    let error = ControlServer::bind(
        fixture.repo.path(),
        "aa-1",
        Some(fixture.runtime.path().as_os_str()),
    )
    .err()
    .unwrap();
    assert!(error
        .to_string()
        .contains("existing endpoints are not automatically removed"));
    assert!(path.exists());
}

#[test]
fn bind_rejects_insecure_or_symlinked_runtime_descendants() {
    let fixture = Fixture::new();
    let parent = fixture.server.path.parent().unwrap();
    fs::set_permissions(parent, fs::Permissions::from_mode(0o755)).unwrap();
    assert!(ControlServer::bind(
        fixture.repo.path(),
        "aa-2",
        Some(fixture.runtime.path().as_os_str())
    )
    .is_err());
    fs::set_permissions(parent, fs::Permissions::from_mode(0o700)).unwrap();
    let runtime = tempfile::tempdir().unwrap();
    fs::set_permissions(runtime.path(), fs::Permissions::from_mode(0o700)).unwrap();
    std::os::unix::fs::symlink(
        fixture.runtime.path().join("waap"),
        runtime.path().join("waap"),
    )
    .unwrap();
    assert!(ControlServer::bind(
        fixture.repo.path(),
        "aa-2",
        Some(runtime.path().as_os_str())
    )
    .is_err());
}

pub(in crate::agent) fn assert_backend_socket_rejection(
    steer: impl Fn(&str, Instant) -> Result<(), SteeringError> + Sync,
    backend: &str,
) {
    struct Run<F> {
        steer: F,
        completion: Mutex<Receiver<()>>,
    }
    impl<F: Fn(&str, Instant) -> Result<(), SteeringError> + Sync> RunHandle for Run<F> {
        fn wait(&self) -> io::Result<RunOutcome> {
            self.completion
                .lock()
                .unwrap()
                .recv_timeout(Duration::from_secs(5))
                .unwrap();
            Ok(RunOutcome::Completed)
        }
        fn steer(&self, prompt: &str, deadline: Instant) -> Result<(), SteeringError> {
            (self.steer)(prompt, deadline)
        }
    }
    let fixture = Fixture::new();
    let path = fixture.server.path.clone();
    let (complete, completion) = mpsc::channel();
    let client = thread::spawn(move || {
        let mut stream = UnixStream::connect(path).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(3)))
            .unwrap();
        stream.write_all(&frame(1, "steer")).unwrap();
        let result = read_response(stream);
        complete.send(()).unwrap();
        result
    });
    fixture
        .server
        .wait(&Run {
            steer,
            completion: Mutex::new(completion),
        })
        .unwrap();
    let value = client.join().unwrap();
    assert_error(&value, json!(false), "not_implemented");
    assert_eq!(
        value["error"]["message"],
        format!("steering not implemented for backend {backend}")
    );
}
