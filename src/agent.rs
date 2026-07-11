use std::fs;
use std::io;
use std::path::{Path, PathBuf};
#[cfg(test)]
use std::sync::Mutex;

use ::toml::Value;
use clap::ValueEnum;
use serde_json::json;

use crate::frontmatter::{
    invalid_frontmatter_error, parse_frontmatter, reject_unknown_fields, require_datetime,
    require_optional_string, require_optional_string_choice, require_string_choice,
    serialize_record,
};
use crate::ids::{available_record_id, is_record_id};
use crate::record::{markdown_body_after_frontmatter, WaapRecordKind};
use crate::toml::{datetime_string, toml_string};

mod backend;
mod claude;
mod codex;
mod get;
mod list;
mod new;
mod opencode;
mod run;
mod stop;
mod update;

#[cfg(test)]
static OPENCODE_ENV_LOCK: Mutex<()> = Mutex::new(());

pub(crate) use get::{load_agent_content, load_agent_report, print_agent_content_report};
pub(crate) use list::{list_agents, print_agent_list};
pub(crate) use new::{create_agent, print_created_agent_report};
pub(crate) use run::run_agent;
pub(crate) use stop::{print_agent_stop_report, stop_agents_with_systems};
pub(crate) use update::{print_updated_agent_report, update_agent};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct AgentMetadata {
    pub(crate) name: Option<String>,
    pub(crate) creation_date: String,
    pub(crate) status: String,
    pub(crate) session_id: Option<String>,
    pub(crate) system: Option<AgentSystem>,
}

impl AgentMetadata {
    pub(crate) fn from_frontmatter(value: &Value, path: &Path) -> Result<Self, Vec<String>> {
        let mut errors = Vec::new();
        reject_unknown_fields(
            value,
            &[
                "name",
                "creation_date",
                "role",
                "status",
                "session_id",
                "system",
            ],
            path,
            &mut errors,
        );
        require_optional_string(value, "name", path, &mut errors);
        require_datetime(value, "creation_date", path, &mut errors);
        // `role` is a deprecated field; tolerate it when present for backward compatibility.
        require_optional_string(value, "role", path, &mut errors);
        require_string_choice(value, "status", &AgentStatus::labels(), path, &mut errors);
        require_optional_string(value, "session_id", path, &mut errors);
        require_optional_string_choice(value, "system", &AgentSystem::labels(), path, &mut errors);
        if !errors.is_empty() {
            return Err(errors);
        }
        Ok(Self {
            name: value
                .get("name")
                .and_then(Value::as_str)
                .map(str::to_string),
            creation_date: datetime_string(value, "creation_date"),
            status: value
                .get("status")
                .and_then(Value::as_str)
                .expect("validated status")
                .to_string(),
            session_id: value
                .get("session_id")
                .and_then(Value::as_str)
                .map(str::to_string),
            system: value
                .get("system")
                .and_then(Value::as_str)
                .and_then(AgentSystem::parse),
        })
    }

    fn to_frontmatter_lines(&self) -> String {
        let mut lines = String::new();
        if let Some(name) = &self.name {
            lines.push_str(&format!("name = {}\n", toml_string(name)));
        }
        lines.push_str(&format!("creation_date = {}\n", self.creation_date));
        lines.push_str(&format!("status = {}\n", toml_string(&self.status)));
        if let Some(session_id) = &self.session_id {
            lines.push_str(&format!("session_id = {}\n", toml_string(session_id)));
        }
        if let Some(system) = &self.system {
            lines.push_str(&format!("system = {}\n", toml_string(system.as_str())));
        }
        lines
    }
}

pub(crate) fn agent_path(waap_root: &Path, agent_id: &str) -> PathBuf {
    WaapRecordKind::Agent
        .root_path(waap_root)
        .join(agent_id)
        .join("agent.md")
}

pub(crate) fn load_agent_metadata(waap_root: &Path, agent_id: &str) -> io::Result<AgentMetadata> {
    let path = validate_agent_path(waap_root, agent_id)?;
    let mut errors = Vec::new();
    let Some(value) = parse_frontmatter(&path, &mut errors) else {
        return Err(invalid_frontmatter_error(errors));
    };
    AgentMetadata::from_frontmatter(&value, &path).map_err(invalid_frontmatter_error)
}

pub(crate) fn read_agent_record(
    waap_root: &Path,
    agent_id: &str,
) -> io::Result<(AgentMetadata, String)> {
    let path = validate_agent_path(waap_root, agent_id)?;
    let contents = fs::read_to_string(&path)?;
    let metadata = load_agent_metadata(waap_root, agent_id)?;
    let body = markdown_body_after_frontmatter(&contents)?;
    Ok((metadata, body))
}

fn validate_agent_path(waap_root: &Path, agent_id: &str) -> io::Result<PathBuf> {
    if !is_agent_id(agent_id) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("{agent_id:?} is not a valid agent id"),
        ));
    }
    let path = agent_path(waap_root, agent_id);
    if !path.is_file() {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            format!("{} does not exist", path.display()),
        ));
    }
    Ok(path)
}

pub(crate) fn write_agent_record(
    waap_root: &Path,
    agent_id: &str,
    metadata: &AgentMetadata,
    body: &str,
) -> io::Result<()> {
    let path = agent_path(waap_root, agent_id);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let contents = serialize_record(&metadata.to_frontmatter_lines(), body);
    fs::write(path, contents)
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct AgentReport {
    pub(crate) agent_id: String,
    pub(crate) path: PathBuf,
    pub(crate) metadata: AgentMetadata,
    pub(crate) file_size: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
pub(crate) enum AgentStatus {
    Ready,
    Running,
    Completed,
    Failed,
    Aborted,
}

impl AgentStatus {
    pub(crate) fn as_str(&self) -> &'static str {
        match self {
            AgentStatus::Ready => "ready",
            AgentStatus::Running => "running",
            AgentStatus::Completed => "completed",
            AgentStatus::Failed => "failed",
            AgentStatus::Aborted => "aborted",
        }
    }

    pub(crate) fn parse(label: &str) -> Option<Self> {
        Self::value_variants()
            .iter()
            .find(|status| status.as_str() == label)
            .copied()
    }

    fn labels() -> Vec<&'static str> {
        Self::value_variants().iter().map(Self::as_str).collect()
    }

    pub(crate) fn validate_transition(self, next: Self) -> io::Result<()> {
        let allowed = matches!(
            (self, next),
            (Self::Ready, Self::Running | Self::Aborted)
                | (
                    Self::Running,
                    Self::Completed | Self::Failed | Self::Aborted
                )
        );
        if allowed {
            Ok(())
        } else {
            Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!(
                    "invalid agent status transition: {} -> {}",
                    self.as_str(),
                    next.as_str()
                ),
            ))
        }
    }
}

pub(crate) fn transition_agent_status(
    metadata: &mut AgentMetadata,
    next: AgentStatus,
) -> io::Result<()> {
    let current = AgentStatus::parse(&metadata.status).ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            format!("invalid agent status {:?}", metadata.status),
        )
    })?;
    current.validate_transition(next)?;
    metadata.status = next.as_str().to_string();
    Ok(())
}

#[derive(Clone, Debug, Default, PartialEq, Eq, ValueEnum)]
pub(crate) enum AgentSystem {
    #[default]
    Opencode,
    Claude,
    Codex,
}

impl AgentSystem {
    fn backend(&self) -> io::Result<Box<dyn backend::AgentSystemBackend>> {
        match self {
            AgentSystem::Opencode => Ok(Box::new(opencode::OpencodeBackend::from_env()?)),
            AgentSystem::Claude => Ok(Box::new(claude::ClaudeBackend::from_env())),
            AgentSystem::Codex => Ok(Box::new(codex::CodexBackend::from_env())),
        }
    }

    pub(crate) fn as_str(&self) -> &'static str {
        match self {
            AgentSystem::Opencode => "opencode",
            AgentSystem::Claude => "claude",
            AgentSystem::Codex => "codex",
        }
    }

    pub(crate) fn parse(label: &str) -> Option<AgentSystem> {
        AgentSystem::value_variants()
            .iter()
            .find(|system| system.as_str() == label)
            .cloned()
    }

    fn labels() -> Vec<&'static str> {
        AgentSystem::value_variants()
            .iter()
            .map(AgentSystem::as_str)
            .collect()
    }
}

pub(crate) fn available_agent_id(agents_dir: &Path, name: Option<&str>) -> io::Result<String> {
    available_record_id(agents_dir, "aa-", name)
}

pub(crate) fn is_agent_id(value: &str) -> bool {
    is_record_id(value, "aa-")
}

pub(crate) fn print_agent_report_human(header: &str, report: &AgentReport) {
    println!("{header} {}", report.agent_id);
    println!("Path: {}", report.path.display());
    if let Some(name) = &report.metadata.name {
        println!("Name: {name}");
    }
    println!("Creation date: {}", report.metadata.creation_date);
    println!("Status: {}", report.metadata.status);
    if let Some(session_id) = &report.metadata.session_id {
        println!("Session ID: {session_id}");
    }
    println!("File size: {} bytes", report.file_size);
}

pub(crate) fn agent_report_json(report: &AgentReport) -> serde_json::Value {
    json!({
        "agent_id": report.agent_id,
        "path": report.path.display().to_string(),
        "metadata": {
            "name": report.metadata.name,
            "creation_date": report.metadata.creation_date,
            "status": report.metadata.status,
            "session_id": report.metadata.session_id,
        },
        "file_size": report.file_size,
    })
}

#[cfg(test)]
mod tests {
    use std::env;
    use std::path::PathBuf;

    use serde_json::json;

    use super::{
        agent_report_json, is_agent_id, transition_agent_status, AgentMetadata, AgentReport,
        AgentStatus, AgentSystem, OPENCODE_ENV_LOCK,
    };
    use crate::ids::random_hex_chars;

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
        let toml =
            "creation_date = 2026-06-18T15:00:34Z\nstatus = \"failed\"\nsystem = \"codex\"\n";
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
            let backend = system.backend().unwrap();
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

        AgentSystem::Claude.backend().unwrap();
        AgentSystem::Codex.backend().unwrap();

        for (name, value) in names.into_iter().zip(previous) {
            if let Some(value) = value {
                env::set_var(name, value);
            }
        }
    }

    #[test]
    fn agent_report_json_has_expected_shape() {
        let report = AgentReport {
            agent_id: "aa-3881fda0".to_string(),
            path: PathBuf::from(".waap/agents/aa-3881fda0/agent.md"),
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
                "path": ".waap/agents/aa-3881fda0/agent.md",
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
}
