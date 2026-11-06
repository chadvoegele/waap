use std::collections::BTreeMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::Path;
use std::thread;

use serde_json::json;
use tempfile::tempdir;

use super::{
    abort_opencode_session, create_session_payload, monitor_opencode_events,
    opencode_directory_query, opencode_repository_root, opencode_url, parse_opencode_model,
    prompt_payload, OpencodeBackend, OpencodeRunConfig, RunOutcome, StartContext,
};

use crate::agent::commands::CommandDispatcher;
use crate::agent::commands::CommandError;
use futures_util::FutureExt;
use std::time::{Duration, Instant};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

async fn pending_runner() -> (super::OpencodeRunner, tokio::net::TcpStream) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let (response, socket) = tokio::join!(
        super::opencode_client()
            .unwrap()
            .get(format!("http://{address}/event"))
            .send(),
        async {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = Vec::new();
            while !request.ends_with(b"\r\n\r\n") {
                request.push(socket.read_u8().await.unwrap());
            }
            socket.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nConnection: close\r\n\r\n: connected\n\n").await.unwrap();
            socket
        }
    );
    (
        super::OpencodeRunner {
            events: Some(response.unwrap()),
            session_id: "ses_test".into(),
        },
        socket,
    )
}

async fn finish_events(socket: &mut tokio::net::TcpStream) {
    socket.write_all(b"data: {\"type\":\"session.status\",\"properties\":{\"sessionID\":\"ses_test\",\"status\":{\"type\":\"idle\"}}}\n\n").await.unwrap();
    socket.shutdown().await.unwrap();
}

#[tokio::test(flavor = "current_thread")]
async fn concrete_runner_rejects_steering_then_finishes_the_same_event_wait() {
    let (mut runner, mut socket) = pending_runner().await;
    let (dispatcher, receiver) = CommandDispatcher::create_channel();
    let mut execution = Box::pin(runner.run(receiver));
    assert!(execution.as_mut().now_or_never().is_none());
    let ((), result) = tokio::join!(
        async {
            assert_eq!(
                dispatcher
                    .steer("pending", Instant::now() + Duration::from_secs(2))
                    .await,
                Err(CommandError::not_implemented("opencode", "steering"))
            );
            finish_events(&mut socket).await;
        },
        tokio::time::timeout(Duration::from_secs(2), &mut execution)
    );
    assert_eq!(result.unwrap().unwrap(), RunOutcome::Completed);
    drop(execution);
    assert!(runner.events.is_none());
    assert!(matches!(
        dispatcher
            .steer("completed", Instant::now() + Duration::from_secs(2))
            .await,
        Err(CommandError::Rejected {
            code: "agent_exited",
            ..
        })
    ));
    let mut byte = [0];
    assert_eq!(
        tokio::time::timeout(Duration::from_secs(2), socket.read(&mut byte))
            .await
            .unwrap()
            .unwrap(),
        0
    );
}

#[tokio::test(flavor = "current_thread")]
async fn concrete_runner_completion_releases_queued_replies() {
    let (mut runner, mut socket) = pending_runner().await;
    finish_events(&mut socket).await;
    let (dispatcher, receiver) = CommandDispatcher::create_channel();
    let steering = dispatcher.steer("queued", Instant::now() + Duration::from_secs(30));
    tokio::pin!(steering);
    assert!(steering.as_mut().now_or_never().is_none());
    assert_eq!(
        tokio::time::timeout(Duration::from_secs(2), runner.run(receiver))
            .await
            .unwrap()
            .unwrap(),
        RunOutcome::Completed
    );
    let result = tokio::time::timeout(Duration::from_millis(100), steering)
        .await
        .unwrap();
    assert!(
        matches!(result, Err(CommandError::AcceptanceUnknown { .. }))
            || result == Err(CommandError::not_implemented("opencode", "steering"))
    );
    assert!(runner.events.is_none());
    let mut byte = [0];
    assert_eq!(
        tokio::time::timeout(Duration::from_secs(2), socket.read(&mut byte))
            .await
            .unwrap()
            .unwrap(),
        0
    );
}

#[tokio::test(flavor = "current_thread")]
async fn concrete_runner_cancellation_releases_queued_replies_before_or_after_polling() {
    for polled in [false, true] {
        let (mut runner, mut socket) = pending_runner().await;
        let (dispatcher, receiver) = CommandDispatcher::create_channel();
        let mut execution = Box::pin(runner.run(receiver));
        if polled {
            assert!(execution.as_mut().now_or_never().is_none());
        }
        let steering = dispatcher.steer("queued", Instant::now() + Duration::from_secs(30));
        tokio::pin!(steering);
        assert!(steering.as_mut().now_or_never().is_none());
        drop(execution);
        assert!(matches!(
            tokio::time::timeout(Duration::from_millis(100), steering)
                .await
                .unwrap(),
            Err(CommandError::AcceptanceUnknown { .. })
        ));
        assert!(matches!(
            dispatcher
                .steer("cancelled", Instant::now() + Duration::from_secs(2))
                .await,
            Err(CommandError::Rejected {
                code: "agent_exited",
                ..
            })
        ));
        if polled {
            assert!(runner.events.is_none());
        }
        drop(runner);
        let mut byte = [0];
        assert_eq!(
            tokio::time::timeout(Duration::from_secs(2), socket.read(&mut byte))
                .await
                .unwrap()
                .unwrap(),
            0
        );
    }
}

#[test]
fn opencode_create_session_payload_matches_api() {
    assert_eq!(
        create_session_payload(),
        json!({
            "permission": [
                { "permission": "question", "action": "deny", "pattern": "*" },
                { "permission": "plan_enter", "action": "deny", "pattern": "*" },
                { "permission": "plan_exit", "action": "deny", "pattern": "*" },
            ]
        })
    );
}

#[test]
fn opencode_model_parsing_supports_variants_and_nested_model_ids() {
    assert_eq!(
        parse_opencode_model("openai/gpt-5.5/high").unwrap(),
        super::OpencodeModel {
            provider_id: "openai".to_string(),
            model_id: "gpt-5.5".to_string(),
            variant: Some("high".to_string()),
        }
    );
    assert_eq!(
        parse_opencode_model("openai/gpt-5.5").unwrap(),
        super::OpencodeModel {
            provider_id: "openai".to_string(),
            model_id: "gpt-5.5".to_string(),
            variant: None,
        }
    );
    assert_eq!(
        parse_opencode_model("openrouter/anthropic/claude-sonnet-4").unwrap(),
        super::OpencodeModel {
            provider_id: "openrouter".to_string(),
            model_id: "anthropic/claude-sonnet-4".to_string(),
            variant: None,
        }
    );
    for invalid in [
        "openai",
        "/gpt-5.5",
        "openai/",
        "openai/gpt-5.5/",
        "openai/gpt-5.5/ ",
        "openai/gpt-5.5/ high",
        " openai/gpt-5.5",
    ] {
        assert!(parse_opencode_model(invalid).is_err(), "{invalid}");
    }
}

#[test]
fn opencode_prompt_payload_matches_async_api() {
    let config = OpencodeRunConfig::for_test();

    assert_eq!(
        prompt_payload(&config, "complete the work"),
        json!({
            "model": { "providerID": "openai", "modelID": "gpt-5.5" },
            "agent": "build",
            "parts": [{ "type": "text", "text": "complete the work" }],
        })
    );
}

#[test]
fn opencode_prompt_payload_includes_variant_when_configured() {
    let mut config = OpencodeRunConfig::for_test();
    config.model = parse_opencode_model("openai/gpt-5/high").unwrap();

    assert_eq!(
        prompt_payload(&config, "complete the work"),
        json!({
            "model": { "providerID": "openai", "modelID": "gpt-5" },
            "variant": "high",
            "agent": "build",
            "parts": [{ "type": "text", "text": "complete the work" }],
        })
    );
}

#[test]
fn opencode_url_trims_trailing_slash() {
    let mut config = OpencodeRunConfig::for_test();
    config.server_url = "https://opencode.example/".to_string();

    assert_eq!(
        opencode_url(&config, "/session"),
        "https://opencode.example/session"
    );
}

#[test]
fn opencode_session_and_abort_use_canonical_repository_root() {
    let dir = tempdir().unwrap();
    let repository_root = opencode_repository_root(dir.path()).unwrap();

    assert_eq!(repository_root, dir.path().canonicalize().unwrap());
    assert_eq!(
        opencode_directory_query(&repository_root),
        [("directory", repository_root.display().to_string())]
    );
}

#[tokio::test(flavor = "current_thread")]
async fn async_runner_subscribes_before_prompt_and_scopes_every_request() {
    let root = tempdir().unwrap();
    let repository_root = root.path().canonicalize().unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let expected_root = repository_root.clone();
    let (release, completion) = std::sync::mpsc::channel();
    let server = thread::spawn(move || {
        let mut create = listener.accept().unwrap().0;
        let request = read_request(&mut create);
        assert_request(&request, "POST", "/session", &expected_root);
        assert_eq!(request.body, create_session_payload());
        write_response(&mut create, "HTTP/1.1 200 OK", Some(r#"{"id":"ses_123"}"#));
        drop(create);

        let mut events = listener.accept().unwrap().0;
        let request = read_request(&mut events);
        assert_request(&request, "GET", "/event", &expected_root);
        events
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\n\r\n: connected\n\n")
            .unwrap();
        events.flush().unwrap();

        let mut prompt = listener.accept().unwrap().0;
        let request = read_request(&mut prompt);
        assert_request(
            &request,
            "POST",
            "/session/ses_123/prompt_async",
            &expected_root,
        );
        assert_eq!(
            request.body,
            json!({
                "model": { "providerID": "openai", "modelID": "gpt-5.5" },
                "agent": "build",
                "parts": [{ "type": "text", "text": "complete the work" }],
            })
        );
        write_response(&mut prompt, "HTTP/1.1 204 No Content", None);
        drop(prompt);

        completion
            .recv_timeout(std::time::Duration::from_secs(5))
            .unwrap();
        events
            .write_all(
                b"data: {\"type\":\"session.status\",\"properties\":{\"sessionID\":\"ses_123\",\"status\":{\"type\":\"idle\"}}}\n\n",
            )
            .unwrap();
    });
    let mut backend = OpencodeBackend {
        config: Some(OpencodeRunConfig {
            server_url: format!("http://{address}"),
            ..OpencodeRunConfig::for_test()
        }),
    };

    let mut started = backend
        .start(
            StartContext {
                agent_id: "aa-00000001",
                prompt: "complete the work",
                repository_root: &repository_root,
                worktree_dir: root.path(),
            },
            super::super::commands::CommandDispatcher::create_channel().1,
        )
        .await
        .unwrap();

    assert_eq!(started.session_id, "ses_123");
    crate::agent::control::tests::assert_backend_socket_rejection(&mut started.runner, "opencode")
        .await;
    release.send(()).unwrap();
    started.runner.cleanup().await.unwrap();
    server.join().unwrap();
}

#[tokio::test(flavor = "current_thread")]
async fn abort_request_is_authenticated_and_scoped_to_repository_root() {
    let root = tempdir().unwrap();
    let repository_root = root.path().canonicalize().unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let expected_root = repository_root.clone();
    let server = thread::spawn(move || {
        let mut stream = listener.accept().unwrap().0;
        let request = read_request(&mut stream);
        assert_request(&request, "POST", "/session/ses_123/abort", &expected_root);
        write_response(&mut stream, "HTTP/1.1 204 No Content", None);
    });
    let config = OpencodeRunConfig {
        server_url: format!("http://{address}"),
        ..OpencodeRunConfig::for_test()
    };

    abort_opencode_session(&config, "ses_123", &repository_root)
        .await
        .unwrap();
    server.join().unwrap();
}

#[tokio::test(flavor = "current_thread")]
async fn monitor_parses_sse_framing_and_emits_cli_compatible_json_lines() {
    let input = concat!(
        ": connected\n\n",
        "event: message.part.updated\n",
        "data: {\"type\":\"message.part.updated\",\n",
        "data: \"properties\":{\"part\":{\"sessionID\":\"ses_other\",\"type\":\"text\",\"time\":{\"end\":1}}}}\n\n",
        "data: {\"type\":\"message.part.updated\",\"properties\":{\"part\":{\"sessionID\":\"ses_123\",\"type\":\"tool\",\"state\":{\"status\":\"completed\"}}}}\n\n",
        "data: {\"type\":\"message.part.updated\",\"properties\":{\"part\":{\"sessionID\":\"ses_123\",\"type\":\"step-start\"}}}\n\n",
        "data: {\"type\":\"message.part.updated\",\"properties\":{\"part\":{\"sessionID\":\"ses_123\",\"type\":\"step-finish\"}}}\n\n",
        "data: {\"type\":\"message.part.updated\",\"properties\":{\"part\":{\"sessionID\":\"ses_123\",\"type\":\"text\",\"time\":{\"end\":1}}}}\n\n",
        "data: {\"type\":\"session.status\",\"properties\":{\"sessionID\":\"ses_other\",\"status\":{\"type\":\"idle\"}}}\n\n",
        "data: {\"type\":\"session.status\",\"properties\":{\"sessionID\":\"ses_123\",\"status\":{\"type\":\"idle\"}}}\n\n",
    );
    let mut output = Vec::new();

    assert_eq!(
        monitor_opencode_events(
            tokio::io::BufReader::new(input.as_bytes()),
            "ses_123",
            &mut output
        )
        .await
        .unwrap(),
        RunOutcome::Completed
    );
    let lines = String::from_utf8(output)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str::<serde_json::Value>(line).unwrap())
        .collect::<Vec<_>>();

    assert_eq!(
        lines
            .iter()
            .map(|line| line["type"].as_str().unwrap())
            .collect::<Vec<_>>(),
        ["tool_use", "step_start", "step_finish", "text"]
    );
    assert!(lines.iter().all(|line| line["sessionID"] == "ses_123"));
    assert!(lines.iter().all(|line| line["timestamp"].is_u64()));
}

#[tokio::test(flavor = "current_thread")]
async fn monitor_reports_matching_session_errors_as_failed_output() {
    let input = "data: {\"type\":\"session.error\",\"properties\":{\"sessionID\":\"ses_123\",\"error\":{\"name\":\"UnknownError\",\"data\":{\"message\":\"failed\"}}}}\n\n";
    let mut output = Vec::new();

    assert_eq!(
        monitor_opencode_events(
            tokio::io::BufReader::new(input.as_bytes()),
            "ses_123",
            &mut output
        )
        .await
        .unwrap(),
        RunOutcome::Failed(std::process::ExitCode::from(1))
    );
    let event = serde_json::from_slice::<serde_json::Value>(&output).unwrap();
    assert_eq!(event["type"], "error");
    assert!(event["timestamp"].is_u64());
    assert_eq!(event["sessionID"], "ses_123");
    assert_eq!(
        event["error"],
        json!({ "name": "UnknownError", "data": { "message": "failed" } })
    );
}

#[tokio::test(flavor = "current_thread")]
async fn monitor_rejects_malformed_events_and_premature_disconnects() {
    for input in [
        "data: not json\n\n",
        "data: {\"type\":\"session.status\",\"properties\":{\"sessionID\":\"ses_123\"}}\n\n",
        "data: {\"type\":\"session.status\",\"properties\":{\"sessionID\":\"ses_other\",\"status\":{\"type\":\"idle\"}}}\n\n",
        "bad field\n",
    ] {
        let error = monitor_opencode_events(tokio::io::BufReader::new(input.as_bytes()), "ses_123", &mut Vec::new()).await
            .unwrap_err();
        assert!(
            matches!(error.kind(), std::io::ErrorKind::InvalidData | std::io::ErrorKind::UnexpectedEof),
            "{}",
            error
        );
    }
}

#[tokio::test(flavor = "current_thread")]
async fn rejected_event_subscription_fails_before_prompt_submission() {
    let root = tempdir().unwrap();
    let repository_root = root.path().canonicalize().unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let expected_root = repository_root.clone();
    let server = thread::spawn(move || {
        let mut create = listener.accept().unwrap().0;
        let request = read_request(&mut create);
        assert_request(&request, "POST", "/session", &expected_root);
        write_response(&mut create, "HTTP/1.1 200 OK", Some(r#"{"id":"ses_123"}"#));
        drop(create);

        let mut events = listener.accept().unwrap().0;
        let request = read_request(&mut events);
        assert_request(&request, "GET", "/event", &expected_root);
        write_response(
            &mut events,
            "HTTP/1.1 503 Service Unavailable",
            Some("unavailable"),
        );
    });
    let mut backend = OpencodeBackend {
        config: Some(OpencodeRunConfig {
            server_url: format!("http://{address}"),
            ..OpencodeRunConfig::for_test()
        }),
    };

    let error = match backend
        .start(
            StartContext {
                agent_id: "aa-00000001",
                prompt: "complete the work",
                repository_root: &repository_root,
                worktree_dir: root.path(),
            },
            super::super::commands::CommandDispatcher::create_channel().1,
        )
        .await
    {
        Ok(_) => panic!("event subscription unexpectedly succeeded"),
        Err(error) => error,
    };

    assert!(error.to_string().contains("503"));
    server.join().unwrap();
}

struct HttpRequest {
    method: String,
    target: String,
    headers: BTreeMap<String, String>,
    body: serde_json::Value,
}

fn read_request(stream: &mut TcpStream) -> HttpRequest {
    let mut reader = BufReader::new(stream);
    let mut request_line = String::new();
    reader.read_line(&mut request_line).unwrap();
    let mut parts = request_line.split_whitespace();
    let method = parts.next().unwrap().to_string();
    let target = parts.next().unwrap().to_string();
    assert_eq!(parts.next(), Some("HTTP/1.1"));

    let mut headers = BTreeMap::new();
    loop {
        let mut line = String::new();
        reader.read_line(&mut line).unwrap();
        if line == "\r\n" {
            break;
        }
        let (name, value) = line.trim_end().split_once(':').unwrap();
        headers.insert(name.to_ascii_lowercase(), value.trim().to_string());
    }
    let content_length = headers
        .get("content-length")
        .map(|value| value.parse::<usize>().unwrap())
        .unwrap_or(0);
    let mut body = vec![0; content_length];
    reader.read_exact(&mut body).unwrap();

    HttpRequest {
        method,
        target,
        headers,
        body: if body.is_empty() {
            serde_json::Value::Null
        } else {
            serde_json::from_slice(&body).unwrap()
        },
    }
}

fn assert_request(request: &HttpRequest, method: &str, path: &str, repository_root: &Path) {
    assert_eq!(request.method, method);
    let (request_path, query) = request.target.split_once('?').unwrap();
    assert_eq!(request_path, path);
    assert_eq!(
        query_parameter(query, "directory"),
        Some(repository_root.display().to_string())
    );
    assert_eq!(
        request.headers.get("authorization"),
        Some(&"Basic cnVubmVyOnNlY3JldA==".to_string())
    );
}

fn query_parameter(query: &str, name: &str) -> Option<String> {
    query.split('&').find_map(|entry| {
        let (key, value) = entry.split_once('=')?;
        (key == name).then(|| percent_decode(value))
    })
}

fn percent_decode(value: &str) -> String {
    let mut decoded = Vec::new();
    let bytes = value.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        match bytes[index] {
            b'%' if index + 2 < bytes.len() => {
                decoded.push(hex_value(bytes[index + 1]) * 16 + hex_value(bytes[index + 2]));
                index += 3;
            }
            b'+' => {
                decoded.push(b' ');
                index += 1;
            }
            byte => {
                decoded.push(byte);
                index += 1;
            }
        }
    }
    String::from_utf8(decoded).unwrap()
}

fn hex_value(value: u8) -> u8 {
    match value {
        b'0'..=b'9' => value - b'0',
        b'a'..=b'f' => value - b'a' + 10,
        b'A'..=b'F' => value - b'A' + 10,
        _ => panic!("invalid percent encoding"),
    }
}

fn write_response(stream: &mut TcpStream, status: &str, body: Option<&str>) {
    let body = body.unwrap_or_default();
    write!(
        stream,
        "{status}\r\nConnection: close\r\nContent-Length: {}\r\n\r\n{body}",
        body.len()
    )
    .unwrap();
    stream.flush().unwrap();
}
