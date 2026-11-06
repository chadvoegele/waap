use std::env;
use std::path::PathBuf;

use serde_json::json;

use super::{
    agent_report_json, is_agent_id, transition_agent_status, AgentMetadata, AgentReport,
    AgentRunOptions, AgentStatus, AgentSystem, CODEX_ENV_LOCK, OPENCODE_ENV_LOCK,
};
use crate::ids::random_hex_chars;

#[test]
fn read_agent_record_keeps_metadata_and_body_from_one_published_version() {
    let dir = tempfile::tempdir().unwrap();
    let path = super::agent_path(dir.path(), "aa-test");
    let record = |version| {
        format!("+++\nname = \"{version}\"\ncreation_date = 2026-06-18T15:00:34Z\nstatus = \"ready\"\n+++\n{version}")
    };
    crate::state::write_record_atomically(&path, &record("a")).unwrap();
    std::thread::scope(|scope| {
        let path = &path;
        scope.spawn(|| {
            for index in 0..100 {
                let version = if index % 2 == 0 { "a" } else { "b" };
                crate::state::write_record_atomically(path, &record(version)).unwrap();
            }
        });
        for _ in 0..100 {
            let (metadata, body) = super::read_agent_record(dir.path(), "aa-test").unwrap();
            assert_eq!(metadata.name.as_deref(), Some(body.as_str()));
        }
    });
}

#[test]
fn generated_agent_ids_are_prefixed_lowercase_hex() {
    let id = format!("aa-{}", random_hex_chars(8).unwrap());

    assert!(id
        .strip_prefix("aa-")
        .unwrap()
        .bytes()
        .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase()));
    assert!(is_agent_id(&id));
}

#[test]
fn agent_ids_are_slug_style() {
    assert!(is_agent_id("aa-custom-agent123"));

    for value in [
        "",
        "custom-agent",
        "aa-Upper",
        "aa-has space",
        "aa-has/slash",
        "aa-has_underscore",
        &format!("aa-{}", "a".repeat(64)),
    ] {
        assert!(!is_agent_id(value));
    }
}

#[test]
fn agent_metadata_unknown_field_is_error() {
    let path = PathBuf::from("agent.md");
    let toml = "creation_date = 2026-06-18T15:00:34Z\nrole = \"developer\"\nstatus = \"ready\"\nworktree = \"some/path\"\n";
    let value: ::toml::Value = toml.parse().unwrap();

    let errors = AgentMetadata::from_frontmatter(&value, &path)
        .err()
        .unwrap();
    assert!(errors.iter().any(|e| e.contains("unknown field worktree")));
}

#[test]
fn agent_metadata_known_fields_pass() {
    let path = PathBuf::from("agent.md");
    let toml = "name = \"Developer\"\ncreation_date = 2026-06-18T15:00:34Z\nrole = \"developer\"\nstatus = \"ready\"\nsession_id = \"ses_1\"\nsystem = \"claude\"\n";
    let value: ::toml::Value = toml.parse().unwrap();

    let metadata = AgentMetadata::from_frontmatter(&value, &path).unwrap();
    assert_eq!(metadata.name.as_deref(), Some("Developer"));
    assert!(metadata
        .to_frontmatter_lines()
        .starts_with("name = \"Developer\"\n"));
}

#[test]
fn agent_metadata_system_codex_passes() {
    let path = PathBuf::from("agent.md");
    let toml = "creation_date = 2026-06-18T15:00:34Z\nstatus = \"ready\"\nsystem = \"codex\"\n";
    let value: ::toml::Value = toml.parse().unwrap();

    assert!(AgentMetadata::from_frontmatter(&value, &path).is_ok());
}

#[test]
fn agent_metadata_failed_status_passes() {
    let path = PathBuf::from("agent.md");
    let toml = "creation_date = 2026-06-18T15:00:34Z\nstatus = \"failed\"\nsystem = \"codex\"\n";
    let value: ::toml::Value = toml.parse().unwrap();

    assert!(AgentMetadata::from_frontmatter(&value, &path).is_ok());
}

#[test]
fn agent_status_transition_graph_accepts_only_documented_edges() {
    let statuses = [
        AgentStatus::Ready,
        AgentStatus::Running,
        AgentStatus::Completed,
        AgentStatus::Failed,
        AgentStatus::Aborted,
    ];
    let allowed = [
        (AgentStatus::Ready, AgentStatus::Running),
        (AgentStatus::Ready, AgentStatus::Aborted),
        (AgentStatus::Running, AgentStatus::Completed),
        (AgentStatus::Running, AgentStatus::Failed),
        (AgentStatus::Running, AgentStatus::Aborted),
    ];

    for current in statuses {
        for next in statuses {
            let expected = allowed.contains(&(current, next));
            assert_eq!(
                current.validate_transition(next).is_ok(),
                expected,
                "{} -> {}",
                current.as_str(),
                next.as_str()
            );
        }
    }
}

#[test]
fn transition_agent_status_updates_only_allowed_transitions() {
    let mut metadata = AgentMetadata {
        name: None,
        creation_date: "2026-06-18T15:00:34Z".to_string(),
        status: "ready".to_string(),
        session_id: None,
        system: None,
    };

    transition_agent_status(&mut metadata, AgentStatus::Running).unwrap();
    assert_eq!(metadata.status, "running");

    let error = transition_agent_status(&mut metadata, AgentStatus::Ready).unwrap_err();
    assert_eq!(error.kind(), std::io::ErrorKind::InvalidInput);
    assert_eq!(metadata.status, "running");
}

#[test]
fn agent_system_codex_round_trips() {
    assert_eq!(AgentSystem::parse("codex"), Some(AgentSystem::Codex));
    assert_eq!(AgentSystem::Codex.as_str(), "codex");
    assert!(AgentSystem::labels().contains(&"codex"));
}

#[test]
fn agent_system_constructs_each_selected_backend() {
    let _lock = OPENCODE_ENV_LOCK.lock().unwrap();
    let names = [
        "OPENCODE_SERVER_URL",
        "OPENCODE_SERVER_USERNAME",
        "OPENCODE_SERVER_PASSWORD",
        "OPENCODE_SERVER_MODEL",
    ];
    let previous = names.map(env::var_os);
    for name in names {
        env::set_var(
            name,
            if name == "OPENCODE_SERVER_MODEL" {
                "test-provider/test-model"
            } else {
                "test-value"
            },
        );
    }

    for (system, expected_type) in [
        (AgentSystem::Opencode, "agent::opencode::OpencodeBackend"),
        (AgentSystem::Claude, "agent::claude::ClaudeBackend"),
        (AgentSystem::Codex, "agent::codex::CodexBackend"),
    ] {
        let backend = system.backend(None).unwrap();
        assert!(backend.type_name().ends_with(expected_type));
    }

    for (name, value) in names.into_iter().zip(previous) {
        match value {
            Some(value) => env::set_var(name, value),
            None => env::remove_var(name),
        }
    }
}

#[test]
fn claude_and_codex_backend_construction_ignores_opencode_environment() {
    let _lock = OPENCODE_ENV_LOCK.lock().unwrap();
    let names = [
        "OPENCODE_SERVER_URL",
        "OPENCODE_SERVER_USERNAME",
        "OPENCODE_SERVER_PASSWORD",
        "OPENCODE_SERVER_MODEL",
    ];
    let previous = names.map(env::var_os);
    for name in names {
        env::remove_var(name);
    }

    AgentSystem::Claude.backend(None).unwrap();
    AgentSystem::Codex.backend(None).unwrap();

    for (name, value) in names.into_iter().zip(previous) {
        if let Some(value) = value {
            env::set_var(name, value);
        }
    }
}

#[test]
fn codex_backend_for_run_does_not_require_opencode_environment() {
    let _opencode_lock = OPENCODE_ENV_LOCK.lock().unwrap();
    let _codex_lock = CODEX_ENV_LOCK.lock().unwrap();
    let opencode_names = [
        "OPENCODE_SERVER_URL",
        "OPENCODE_SERVER_USERNAME",
        "OPENCODE_SERVER_PASSWORD",
        "OPENCODE_SERVER_MODEL",
    ];
    let previous_opencode = opencode_names.map(env::var_os);
    let previous_effort = env::var_os("CODEX_REASONING_EFFORT");
    for name in opencode_names {
        env::remove_var(name);
    }
    env::remove_var("CODEX_REASONING_EFFORT");

    AgentSystem::Codex
        .backend(Some(&AgentRunOptions::default()))
        .unwrap();

    for (name, value) in opencode_names.into_iter().zip(previous_opencode) {
        if let Some(value) = value {
            env::set_var(name, value);
        }
    }
    if let Some(value) = previous_effort {
        env::set_var("CODEX_REASONING_EFFORT", value);
    }
}

#[test]
fn agent_report_json_has_expected_shape() {
    let report = AgentReport {
        agent_id: "aa-3881fda0".to_string(),
        path: PathBuf::from("agents/aa-3881fda0/agent.md"),
        metadata: AgentMetadata {
            name: None,
            creation_date: "2026-06-18T15:00:34Z".to_string(),
            status: "running".to_string(),
            session_id: Some("ses_123".to_string()),
            system: None,
        },
        file_size: 456,
    };

    assert_eq!(
        agent_report_json(&report),
        json!({
            "agent_id": "aa-3881fda0",
            "path": "agents/aa-3881fda0/agent.md",
            "metadata": {
                "name": null,
                "creation_date": "2026-06-18T15:00:34Z",
                "status": "running",
                "session_id": "ses_123",
            },
            "file_size": 456,
        })
    );
}
