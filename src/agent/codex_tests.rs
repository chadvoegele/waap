use std::io::Cursor;
use std::path::PathBuf;

use clap::ValueEnum;

use super::*;
use crate::agent::CODEX_ENV_LOCK;

fn parse(line: &str) -> JsonValue {
    serde_json::from_str(line).expect("line is JSON")
}

fn client_over(
    input: &str,
) -> CodexClient<tokio::io::BufReader<Cursor<Vec<u8>>>, Vec<u8>, Vec<u8>> {
    CodexClient {
        reader: tokio::io::BufReader::new(Cursor::new(input.as_bytes().to_vec())),
        writer: Vec::new(),
        out: Vec::new(),
        next_id: 0,
        model: None,
        reasoning_effort: None,
    }
}

#[test]
fn request_line_is_single_line_with_id_method_params_and_no_jsonrpc() {
    let line = request_line(7, METHOD_INITIALIZE, initialize_params());

    assert!(line.ends_with('\n'));
    assert_eq!(line.matches('\n').count(), 1);

    let value = parse(&line);
    assert_eq!(value["id"], json!(7));
    assert_eq!(value["method"], json!("initialize"));
    assert!(value.get("params").is_some());
    assert!(value.get("jsonrpc").is_none());
}

#[test]
fn notification_line_has_no_id_and_no_jsonrpc() {
    let line = notification_line(METHOD_INITIALIZED);

    assert!(line.ends_with('\n'));
    let value = parse(&line);
    assert_eq!(value["method"], json!("initialized"));
    assert!(value.get("id").is_none());
    assert!(value.get("params").is_none());
    assert!(value.get("jsonrpc").is_none());
}

#[test]
fn initialize_correlates_response_forwards_delta_and_notifies_server() {
    let input = concat!(
        "{\"id\":9,\"result\":{}}\n",
        "{\"method\":\"item/agentMessage/delta\",\"params\":{\"delta\":\"ready\"}}\n",
        "{\"id\":0,\"result\":{}}\n",
    );
    let mut client = client_over(input);

    crate::agent::runtime::block_on(client.initialize()).unwrap();

    assert_eq!(String::from_utf8(client.out.clone()).unwrap(), "ready");
    let lines = String::from_utf8(client.writer.clone()).unwrap();
    let mut lines = lines.lines();
    let request = parse(lines.next().unwrap());
    assert_eq!(request["id"], json!(0));
    assert_eq!(request["method"], json!("initialize"));
    assert!(request.get("params").is_some());
    let notification = parse(lines.next().unwrap());
    assert_eq!(notification, json!({ "method": "initialized" }));
    assert!(lines.next().is_none());
}

#[test]
fn thread_start_params_encode_never_approvals_and_full_access() {
    let worktree_dir = PathBuf::from("/repo/with space");
    let params = thread_start_params(&worktree_dir, Some("o3"));
    let command = codex_app_server_command(&worktree_dir);

    assert_eq!(params["approvalPolicy"], json!("never"));
    assert_eq!(params["sandbox"], json!("danger-full-access"));
    assert_eq!(params["cwd"], json!("/repo/with space"));
    assert_eq!(params["model"], json!("o3"));
    assert!(params.get("effort").is_none());
    assert!(params.get("reasoningEffort").is_none());
    assert_eq!(command.as_std().get_program(), "codex");
    assert_eq!(
        command.as_std().get_args().collect::<Vec<_>>(),
        ["app-server", "--stdio"]
    );
    assert_eq!(
        command.as_std().get_current_dir(),
        Some(worktree_dir.as_path())
    );
}

#[test]
fn thread_start_params_omit_model_when_unset() {
    let params = thread_start_params(&PathBuf::from("/repo"), None);

    assert!(params.get("model").is_none());
    assert_eq!(params["approvalPolicy"], json!("never"));
    assert_eq!(params["sandbox"], json!("danger-full-access"));
}

#[test]
fn turn_start_params_use_camel_case_thread_id_and_carry_prompt() {
    let params = turn_start_params(
        "th_1",
        "do the thing",
        Some("gpt-5.4"),
        Some(ReasoningEffort::High),
    );

    assert_eq!(params["threadId"], json!("th_1"));
    assert_eq!(params["input"][0]["type"], json!("text"));
    assert_eq!(params["input"][0]["text"], json!("do the thing"));
    assert_eq!(params["model"], json!("gpt-5.4"));
    assert_eq!(params["effort"], json!("high"));
    assert!(params.get("reasoningEffort").is_none());
}

#[test]
fn turn_start_params_omit_model_when_unset() {
    let params = turn_start_params("th_1", "prompt", None, None);

    assert!(params.get("model").is_none());
    assert!(params.get("effort").is_none());
    assert_eq!(params["threadId"], json!("th_1"));
}

#[test]
fn turn_start_params_support_model_and_effort_independently() {
    let model_only = turn_start_params("th", "prompt", Some("gpt-5.4"), None);
    let effort_only = turn_start_params("th", "prompt", None, Some(ReasoningEffort::Ultra));

    assert_eq!(model_only["model"], json!("gpt-5.4"));
    assert!(model_only.get("effort").is_none());
    assert!(effort_only.get("model").is_none());
    assert_eq!(effort_only["effort"], json!("ultra"));
}

#[test]
fn codex_config_uses_environment_fallbacks_and_cli_precedence() {
    let _lock = CODEX_ENV_LOCK.lock().unwrap();
    let previous_model = env::var_os("CODEX_MODEL");
    let previous_effort = env::var_os("CODEX_REASONING_EFFORT");
    env::set_var("CODEX_MODEL", "env-model");
    env::set_var("CODEX_REASONING_EFFORT", "low");

    let fallback = get_codex_run_config(&AgentRunOptions::default()).unwrap();
    let overrides = get_codex_run_config(&AgentRunOptions {
        model: Some("cli-model".to_string()),
        reasoning_effort: Some(ReasoningEffort::High),
    })
    .unwrap();
    env::set_var("CODEX_REASONING_EFFORT", "invalid-but-overridden");
    let invalid_override = get_codex_run_config(&AgentRunOptions {
        model: None,
        reasoning_effort: Some(ReasoningEffort::Max),
    })
    .unwrap();

    assert_eq!(fallback.model.as_deref(), Some("env-model"));
    assert_eq!(fallback.reasoning_effort, Some(ReasoningEffort::Low));
    assert_eq!(overrides.model.as_deref(), Some("cli-model"));
    assert_eq!(overrides.reasoning_effort, Some(ReasoningEffort::High));
    assert_eq!(
        invalid_override.reasoning_effort,
        Some(ReasoningEffort::Max)
    );

    match previous_model {
        Some(value) => env::set_var("CODEX_MODEL", value),
        None => env::remove_var("CODEX_MODEL"),
    }
    match previous_effort {
        Some(value) => env::set_var("CODEX_REASONING_EFFORT", value),
        None => env::remove_var("CODEX_REASONING_EFFORT"),
    }
}

#[test]
fn codex_config_accepts_exact_efforts_and_rejects_invalid_environment_values() {
    let _lock = CODEX_ENV_LOCK.lock().unwrap();
    let previous_model = env::var_os("CODEX_MODEL");
    let previous_effort = env::var_os("CODEX_REASONING_EFFORT");
    env::remove_var("CODEX_MODEL");

    for expected in ReasoningEffort::value_variants() {
        env::set_var("CODEX_REASONING_EFFORT", expected.as_str());
        let config = get_codex_run_config(&AgentRunOptions::default()).unwrap();
        assert_eq!(config.reasoning_effort, Some(*expected));
    }
    for invalid in ["", "HIGH", "extreme"] {
        env::set_var("CODEX_REASONING_EFFORT", invalid);
        let error = get_codex_run_config(&AgentRunOptions::default()).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
        let message = error.to_string();
        assert!(message.contains("CODEX_REASONING_EFFORT"));
        for accepted in ReasoningEffort::labels() {
            assert!(message.contains(accepted));
        }
    }

    env::remove_var("CODEX_REASONING_EFFORT");
    assert_eq!(
        get_codex_run_config(&AgentRunOptions::default()).unwrap(),
        CodexRunConfig::default()
    );

    match previous_model {
        Some(value) => env::set_var("CODEX_MODEL", value),
        None => env::remove_var("CODEX_MODEL"),
    }
    match previous_effort {
        Some(value) => env::set_var("CODEX_REASONING_EFFORT", value),
        None => env::remove_var("CODEX_REASONING_EFFORT"),
    }
}

#[test]
fn thread_start_extracts_thread_id_skipping_notifications() {
    let input = concat!(
        "{\"method\":\"thread/started\",\"params\":{\"thread\":{\"id\":\"th_x\"}}}\n",
        "{\"id\":0,\"result\":{\"thread\":{\"id\":\"th_abc\"}}}\n",
    );
    let mut client = client_over(input);

    let thread_id =
        crate::agent::runtime::block_on(client.thread_start(&PathBuf::from("/repo"))).unwrap();

    assert_eq!(thread_id, "th_abc");
    let request = parse(&String::from_utf8(client.writer.clone()).unwrap());
    assert_eq!(request["method"], json!("thread/start"));
}

#[test]
fn thread_start_propagates_server_error() {
    let input = "{\"id\":0,\"error\":{\"code\":-32603,\"message\":\"boom\"}}\n";
    let mut client = client_over(input);

    let error = crate::agent::runtime::block_on(client.thread_start(&PathBuf::from("/repo")))
        .expect_err("server error must propagate");

    assert!(error.to_string().contains("boom"));
}
