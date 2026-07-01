use std::fs;
use std::io;
use std::path::Path;

use serde_json::json;

use crate::agent::{
    agent_path, agent_report_json, load_agent_metadata, print_agent_report_human,
    read_agent_record, AgentReport,
};
use crate::cli::OutputFormat;

pub(crate) fn print_agent_content_report(
    output_format: &OutputFormat,
    report: &AgentReport,
    content: &str,
) {
    match output_format {
        OutputFormat::Json => println!("{}", agent_content_report_json(report, content)),
        OutputFormat::HumanReadable => {
            print_agent_report_human("Agent", report);
            println!("Content:");
            print!("{content}");
        }
    }
}

pub(crate) fn agent_content_report_json(report: &AgentReport, content: &str) -> serde_json::Value {
    let mut value = agent_report_json(report);
    value["content"] = json!(content);
    value
}

pub(crate) fn load_agent_report(waap_root: &Path, agent_id: &str) -> io::Result<AgentReport> {
    let path = agent_path(waap_root, agent_id);
    let metadata = load_agent_metadata(waap_root, agent_id)?;
    let file_size = fs::metadata(&path)?.len();

    Ok(AgentReport {
        agent_id: agent_id.to_string(),
        path,
        metadata,
        file_size,
    })
}

pub(crate) fn load_agent_content(
    waap_root: &Path,
    agent_id: &str,
) -> io::Result<(AgentReport, String)> {
    let report = load_agent_report(waap_root, agent_id)?;
    let (_, body) = read_agent_record(waap_root, agent_id)?;

    Ok((report, body))
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::io;
    use std::path::{Path, PathBuf};

    use serde_json::json;
    use tempfile::tempdir;

    use super::{agent_content_report_json, load_agent_content, load_agent_report};
    use crate::agent::{AgentMetadata, AgentReport};

    #[test]
    fn missing_agent_is_reported() {
        let dir = tempdir().unwrap();

        let error = load_agent_report(dir.path(), "aa-3881fda0").unwrap_err();

        assert_eq!(error.kind(), io::ErrorKind::NotFound);
        assert!(error
            .to_string()
            .contains(".waap/agents/aa-3881fda0/agent.md"));
    }

    #[test]
    fn invalid_agent_frontmatter_is_reported() {
        let dir = tempdir().unwrap();
        write_file(
            &dir.path().join(".waap/agents/aa-3881fda0/agent.md"),
            "+++
creation_date = 2026-06-18T15:00:34Z
role = \"developer\"
status = \"pending\"
+++
",
        );

        let error = load_agent_report(dir.path(), "aa-3881fda0").unwrap_err();

        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
        assert!(error.to_string().contains("status has invalid value"));
    }

    #[test]
    fn load_agent_report_reads_metadata() {
        let dir = tempdir().unwrap();
        let contents = "+++
creation_date = 2026-06-18T15:00:34Z
role = \"developer\"
status = \"ready\"
session_id = \"ses_123\"
+++

# Purpose
";
        write_file(
            &dir.path().join(".waap/agents/aa-3881fda0/agent.md"),
            contents,
        );

        let report = load_agent_report(dir.path(), "aa-3881fda0").unwrap();

        assert_eq!(report.agent_id, "aa-3881fda0");
        assert_eq!(
            report.path,
            dir.path().join(".waap/agents/aa-3881fda0/agent.md")
        );
        assert_eq!(report.metadata.creation_date, "2026-06-18T15:00:34Z");
        assert_eq!(report.metadata.status, "ready");
        assert_eq!(report.metadata.session_id.as_deref(), Some("ses_123"));
        assert_eq!(report.file_size, contents.len() as u64);
    }

    #[test]
    fn load_agent_content_reads_metadata_and_markdown_body() {
        let dir = tempdir().unwrap();
        let contents = "+++
creation_date = 2026-06-18T15:00:34Z
role = \"developer\"
status = \"ready\"
session_id = \"ses_123\"
+++

# Purpose
Do work
";
        write_file(
            &dir.path().join(".waap/agents/aa-3881fda0/agent.md"),
            contents,
        );

        let (report, content) = load_agent_content(dir.path(), "aa-3881fda0").unwrap();

        assert_eq!(report.agent_id, "aa-3881fda0");
        assert_eq!(report.metadata.session_id.as_deref(), Some("ses_123"));
        assert_eq!(content, "\n# Purpose\nDo work\n");
    }

    #[test]
    fn agent_content_report_json_includes_markdown_body() {
        let report = AgentReport {
            agent_id: "aa-3881fda0".to_string(),
            path: PathBuf::from(".waap/agents/aa-3881fda0/agent.md"),
            metadata: AgentMetadata {
                creation_date: "2026-06-18T15:00:34Z".to_string(),
                status: "running".to_string(),
                session_id: Some("ses_123".to_string()),
                system: None,
            },
            file_size: 456,
        };

        assert_eq!(
            agent_content_report_json(&report, "# Purpose\n"),
            json!({
                "agent_id": "aa-3881fda0",
                "path": ".waap/agents/aa-3881fda0/agent.md",
                "metadata": {
                    "creation_date": "2026-06-18T15:00:34Z",
                    "status": "running",
                    "session_id": "ses_123",
                },
                "file_size": 456,
                "content": "# Purpose\n",
            })
        );
    }

    fn write_file(path: &Path, contents: &str) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, contents).unwrap();
    }
}
