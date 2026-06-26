use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use clap::ValueEnum;
use serde_json::json;
use toml::Value;

use crate::frontmatter::{
    datetime_string, invalid_frontmatter_error, parse_frontmatter, parse_frontmatter_from_contents,
    require_datetime, require_optional_string, require_string_choice, serialize_record,
};
use crate::ids::{random_hex_chars, toml_string};
use crate::record::{markdown_body_after_frontmatter, WaapRecordKind};

pub(crate) mod get;
pub(crate) mod list;
pub(crate) mod new;
pub(crate) mod run;
pub(crate) mod stop;
pub(crate) mod update;

pub(crate) use get::{load_agent_content, load_agent_report, print_agent_content_report};
pub(crate) use list::{list_agents, print_agent_list};
pub(crate) use new::{create_agent, print_created_agent_report};
pub(crate) use run::run_agent;
pub(crate) use stop::{print_agent_stop_report, stop_agents_with_opencode};
pub(crate) use update::{print_updated_agent_report, update_agent};

pub(crate) struct AgentMetadata {
    pub(crate) creation_date: String,
    pub(crate) role: String,
    pub(crate) status: String,
    pub(crate) session_id: Option<String>,
}

impl AgentMetadata {
    pub(crate) fn from_frontmatter(value: &Value, path: &Path) -> Result<Self, Vec<String>> {
        let mut errors = Vec::new();
        require_datetime(value, "creation_date", path, &mut errors);
        require_string_choice(value, "role", &["developer", "planner"], path, &mut errors);
        require_string_choice(
            value,
            "status",
            &["ready", "running", "completed", "aborted"],
            path,
            &mut errors,
        );
        require_optional_string(value, "session_id", path, &mut errors);
        if !errors.is_empty() {
            return Err(errors);
        }
        Ok(Self {
            creation_date: datetime_string(value, "creation_date"),
            role: value
                .get("role")
                .and_then(Value::as_str)
                .expect("validated role")
                .to_string(),
            status: value
                .get("status")
                .and_then(Value::as_str)
                .expect("validated status")
                .to_string(),
            session_id: value
                .get("session_id")
                .and_then(Value::as_str)
                .map(str::to_string),
        })
    }

    pub(crate) fn to_frontmatter_lines(&self) -> String {
        let mut lines = String::new();
        lines.push_str(&format!("creation_date = {}\n", self.creation_date));
        lines.push_str(&format!("role = {}\n", toml_string(&self.role)));
        lines.push_str(&format!("status = {}\n", toml_string(&self.status)));
        if let Some(session_id) = &self.session_id {
            lines.push_str(&format!("session_id = {}\n", toml_string(session_id)));
        }
        lines
    }
}

pub(crate) fn agent_path(repo_root: &Path, agent_id: &str) -> PathBuf {
    WaapRecordKind::Agent
        .root_path(repo_root)
        .join(agent_id)
        .join("agent.md")
}

pub(crate) fn load_agent_metadata(repo_root: &Path, agent_id: &str) -> io::Result<AgentMetadata> {
    let path = validate_agent_path(repo_root, agent_id)?;
    let contents = fs::read_to_string(&path)?;
    let mut errors = Vec::new();
    let Some(value) = parse_frontmatter_from_contents(&contents, &path, &mut errors) else {
        return Err(invalid_frontmatter_error(errors));
    };
    AgentMetadata::from_frontmatter(&value, &path).map_err(invalid_frontmatter_error)
}

pub(crate) fn read_agent_record(
    repo_root: &Path,
    agent_id: &str,
) -> io::Result<(AgentMetadata, String)> {
    let path = validate_agent_path(repo_root, agent_id)?;
    let contents = fs::read_to_string(&path)?;
    let mut errors = Vec::new();
    let Some(value) = parse_frontmatter_from_contents(&contents, &path, &mut errors) else {
        return Err(invalid_frontmatter_error(errors));
    };
    let metadata =
        AgentMetadata::from_frontmatter(&value, &path).map_err(invalid_frontmatter_error)?;
    let body = markdown_body_after_frontmatter(&contents)?;
    Ok((metadata, body))
}

fn validate_agent_path(repo_root: &Path, agent_id: &str) -> io::Result<PathBuf> {
    if !is_agent_id(agent_id) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("{agent_id:?} is not a valid agent id"),
        ));
    }
    let path = agent_path(repo_root, agent_id);
    if !path.is_file() {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            format!("{} does not exist", path.display()),
        ));
    }
    Ok(path)
}

pub(crate) fn write_agent_record(
    repo_root: &Path,
    agent_id: &str,
    metadata: &AgentMetadata,
    body: &str,
) -> io::Result<()> {
    let path = agent_path(repo_root, agent_id);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let contents = serialize_record(&metadata.to_frontmatter_lines(), body);
    fs::write(path, contents)
}

pub(crate) fn check_agent_frontmatter(path: &Path, errors: &mut Vec<String>) {
    let Some(frontmatter) = parse_frontmatter(path, errors) else {
        return;
    };
    if let Err(mut frontmatter_errors) = AgentMetadata::from_frontmatter(&frontmatter, path) {
        errors.append(&mut frontmatter_errors);
    }
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct AgentReport {
    pub(crate) agent_id: String,
    pub(crate) path: PathBuf,
    pub(crate) creation_date: String,
    pub(crate) role: String,
    pub(crate) status: String,
    pub(crate) session_id: Option<String>,
    pub(crate) file_size: u64,
}

#[derive(Clone, Debug, ValueEnum)]
pub(crate) enum AgentRole {
    Developer,
    Planner,
}

#[derive(Clone, Debug, ValueEnum)]
pub(crate) enum AgentStatus {
    Ready,
    Running,
    Completed,
    Aborted,
}

impl AgentStatus {
    pub(crate) fn as_str(&self) -> &'static str {
        match self {
            AgentStatus::Ready => "ready",
            AgentStatus::Running => "running",
            AgentStatus::Completed => "completed",
            AgentStatus::Aborted => "aborted",
        }
    }
}

impl AgentRole {
    pub(crate) fn as_str(&self) -> &'static str {
        match self {
            AgentRole::Developer => "developer",
            AgentRole::Planner => "planner",
        }
    }
}

pub(crate) fn available_agent_id(agents_dir: &Path) -> io::Result<String> {
    available_agent_id_with_generator(agents_dir, || random_hex_chars(8))
}

pub(crate) fn available_agent_id_with_generator(
    agents_dir: &Path,
    mut generate_hash: impl FnMut() -> io::Result<String>,
) -> io::Result<String> {
    loop {
        let agent_id = format!("aa-{}", generate_hash()?);
        if !agents_dir.join(&agent_id).exists() {
            return Ok(agent_id);
        }
    }
}

pub(crate) fn is_agent_id(value: &str) -> bool {
    let Some(hash) = value.strip_prefix("aa-") else {
        return false;
    };
    hash.len() == 8
        && hash
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
}

pub(crate) fn print_agent_report_human(header: &str, report: &AgentReport) {
    println!("{header} {}", report.agent_id);
    println!("Path: {}", report.path.display());
    println!("Creation date: {}", report.creation_date);
    println!("Role: {}", report.role);
    println!("Status: {}", report.status);
    if let Some(session_id) = &report.session_id {
        println!("Session ID: {session_id}");
    }
    println!("File size: {} bytes", report.file_size);
}

pub(crate) fn agent_report_json(report: &AgentReport) -> serde_json::Value {
    json!({
        "agent_id": report.agent_id,
        "path": report.path.display().to_string(),
        "metadata": {
            "creation_date": report.creation_date,
            "role": report.role,
            "status": report.status,
            "session_id": report.session_id,
        },
        "file_size": report.file_size,
    })
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::PathBuf;

    use serde_json::json;
    use tempfile::tempdir;

    use super::{agent_report_json, available_agent_id_with_generator, is_agent_id, AgentReport};
    use crate::ids::random_hex_chars;

    #[test]
    fn agent_id_generation_retries_conflicts() {
        let dir = tempdir().unwrap();
        fs::create_dir_all(dir.path().join("aa-00000001")).unwrap();
        let mut hashes = ["00000001", "00000002"].into_iter();

        let agent_id = available_agent_id_with_generator(dir.path(), || {
            Ok(hashes.next().unwrap().to_string())
        })
        .unwrap();

        assert_eq!(agent_id, "aa-00000002");
    }

    #[test]
    fn generated_agent_ids_are_prefixed_lowercase_hex() {
        let id = format!("aa-{}", random_hex_chars(8).unwrap());

        assert!(is_agent_id(&id));
    }

    #[test]
    fn agent_report_json_has_expected_shape() {
        let report = AgentReport {
            agent_id: "aa-3881fda0".to_string(),
            path: PathBuf::from(".waap/agents/aa-3881fda0/agent.md"),
            creation_date: "2026-06-18T15:00:34Z".to_string(),
            role: "developer".to_string(),
            status: "running".to_string(),
            session_id: Some("ses_123".to_string()),
            file_size: 456,
        };

        assert_eq!(
            agent_report_json(&report),
            json!({
                "agent_id": "aa-3881fda0",
                "path": ".waap/agents/aa-3881fda0/agent.md",
                "metadata": {
                    "creation_date": "2026-06-18T15:00:34Z",
                    "role": "developer",
                    "status": "running",
                    "session_id": "ses_123",
                },
                "file_size": 456,
            })
        );
    }
}
