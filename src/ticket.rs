use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use clap::ValueEnum;
use serde_json::json;
use toml::Value;

use crate::frontmatter::{
    datetime_string, invalid_frontmatter_error, parse_frontmatter_from_contents,
    reject_unknown_fields, require_datetime, require_optional_string,
    require_optional_string_array, require_string_choice, serialize_record,
};
use crate::ids::{available_record_id, is_record_id, toml_string};
use crate::record::{markdown_body_after_frontmatter, WaapRecordKind};

pub(crate) mod get;
pub(crate) mod list;
pub(crate) mod new;
pub(crate) mod update;

pub(crate) use get::{get_ticket, print_ticket_get_report};
pub(crate) use list::{list_tickets, print_ticket_list};
pub(crate) use new::{create_ticket, print_ticket_report};
pub(crate) use update::{print_updated_ticket_report, update_ticket};

pub(crate) struct TicketMetadata {
    pub(crate) ticket_id: String,
    pub(crate) name: Option<String>,
    pub(crate) creation_date: String,
    pub(crate) status: String,
    pub(crate) depends_on: Option<Vec<String>>,
}

impl TicketMetadata {
    pub(crate) fn from_frontmatter(
        value: &Value,
        path: &Path,
        ticket_id: &str,
    ) -> Result<Self, Vec<String>> {
        let mut errors = Vec::new();
        reject_unknown_fields(
            value,
            &["name", "title", "creation_date", "status", "depends_on"],
            path,
            &mut errors,
        );
        require_optional_string(value, "name", path, &mut errors);
        // `title` is deprecated; map it to `name` when reading old tickets.
        require_optional_string(value, "title", path, &mut errors);
        require_datetime(value, "creation_date", path, &mut errors);
        require_string_choice(
            value,
            "status",
            &["pending", "in-progress", "completed", "abandoned"],
            path,
            &mut errors,
        );
        require_optional_string_array(value, "depends_on", path, &mut errors);
        if let Some(Value::Array(arr)) = value.get("depends_on") {
            for (i, entry) in arr.iter().enumerate() {
                if let Some(s) = entry.as_str() {
                    if !is_ticket_id(s) {
                        errors.push(format!(
                            "{} frontmatter field depends_on[{i}] {s:?} is not a valid ticket id",
                            path.display()
                        ));
                    }
                }
            }
        }
        if !errors.is_empty() {
            return Err(errors);
        }
        let depends_on = value
            .get("depends_on")
            .and_then(Value::as_array)
            .map(|arr| {
                arr.iter()
                    .filter_map(Value::as_str)
                    .map(str::to_string)
                    .collect::<Vec<_>>()
            })
            .filter(|v| !v.is_empty());
        Ok(Self {
            ticket_id: ticket_id.to_string(),
            name: value
                .get("name")
                .or_else(|| value.get("title"))
                .and_then(Value::as_str)
                .map(str::to_string),
            creation_date: datetime_string(value, "creation_date"),
            status: value
                .get("status")
                .and_then(Value::as_str)
                .expect("validated status")
                .to_string(),
            depends_on,
        })
    }

    pub(crate) fn to_frontmatter_lines(&self) -> String {
        let mut lines = String::new();
        if let Some(name) = &self.name {
            lines.push_str(&format!("name = {}\n", toml_string(name)));
        }
        lines.push_str(&format!("creation_date = {}\n", self.creation_date));
        lines.push_str(&format!("status = {}\n", toml_string(&self.status)));
        if let Some(deps) = &self.depends_on {
            let items: Vec<String> = deps.iter().map(|d| toml_string(d)).collect();
            lines.push_str(&format!("depends_on = [{}]\n", items.join(", ")));
        }
        lines
    }
}

pub(crate) fn ticket_path(waap_root: &Path, ticket_id: &str) -> PathBuf {
    WaapRecordKind::Ticket
        .root_path(waap_root)
        .join(ticket_id)
        .join("ticket.md")
}

pub(crate) fn load_ticket_metadata(
    waap_root: &Path,
    ticket_id: &str,
) -> io::Result<TicketMetadata> {
    let path = validate_ticket_path(waap_root, ticket_id)?;
    let contents = fs::read_to_string(&path)?;
    let mut errors = Vec::new();
    let Some(value) = parse_frontmatter_from_contents(&contents, &path, &mut errors) else {
        return Err(invalid_frontmatter_error(errors));
    };
    TicketMetadata::from_frontmatter(&value, &path, ticket_id).map_err(invalid_frontmatter_error)
}

pub(crate) fn read_ticket_record(
    waap_root: &Path,
    ticket_id: &str,
) -> io::Result<(TicketMetadata, String)> {
    let path = validate_ticket_path(waap_root, ticket_id)?;
    let contents = fs::read_to_string(&path)?;
    let mut errors = Vec::new();
    let Some(value) = parse_frontmatter_from_contents(&contents, &path, &mut errors) else {
        return Err(invalid_frontmatter_error(errors));
    };
    let metadata = TicketMetadata::from_frontmatter(&value, &path, ticket_id)
        .map_err(invalid_frontmatter_error)?;
    let body = markdown_body_after_frontmatter(&contents)?;
    Ok((metadata, body))
}

fn validate_ticket_path(waap_root: &Path, ticket_id: &str) -> io::Result<PathBuf> {
    if !is_ticket_id(ticket_id) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("{ticket_id:?} is not a valid ticket id"),
        ));
    }
    let path = ticket_path(waap_root, ticket_id);
    if !path.is_file() {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            format!("{} does not exist", path.display()),
        ));
    }
    Ok(path)
}

pub(crate) fn write_ticket_record(
    waap_root: &Path,
    ticket_id: &str,
    metadata: &TicketMetadata,
    body: &str,
) -> io::Result<()> {
    let path = ticket_path(waap_root, ticket_id);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let contents = serialize_record(&metadata.to_frontmatter_lines(), body);
    fs::write(path, contents)
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct TicketReport {
    pub(crate) ticket_id: String,
    pub(crate) path: PathBuf,
    pub(crate) name: Option<String>,
    pub(crate) creation_date: String,
    pub(crate) status: String,
    pub(crate) depends_on: Option<Vec<String>>,
    pub(crate) file_size: u64,
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct TicketGetReport {
    pub(crate) ticket: TicketReport,
    pub(crate) content: String,
}

#[derive(Clone, Debug, ValueEnum)]
pub(crate) enum TicketStatus {
    Pending,
    InProgress,
    Completed,
    Abandoned,
}

impl TicketStatus {
    pub(crate) fn as_str(&self) -> &'static str {
        match self {
            TicketStatus::Pending => "pending",
            TicketStatus::InProgress => "in-progress",
            TicketStatus::Completed => "completed",
            TicketStatus::Abandoned => "abandoned",
        }
    }
}

pub(crate) fn available_ticket_id(tickets_dir: &Path, name: Option<&str>) -> io::Result<String> {
    available_record_id(tickets_dir, "tt-", name)
}

pub(crate) fn is_ticket_id(value: &str) -> bool {
    is_record_id(value, "tt-")
}

pub(crate) fn print_ticket_report_human(header: &str, report: &TicketReport) {
    println!("{header} {}", report.ticket_id);
    println!("Path: {}", report.path.display());
    if let Some(name) = &report.name {
        println!("Name: {name}");
    }
    println!("Creation date: {}", report.creation_date);
    println!("Status: {}", report.status);
    if let Some(deps) = &report.depends_on {
        if !deps.is_empty() {
            println!("Depends on: {}", deps.join(", "));
        }
    }
    println!("File size: {} bytes", report.file_size);
}

pub(crate) fn ticket_report_json(report: &TicketReport) -> serde_json::Value {
    let depends_on: serde_json::Value = match &report.depends_on {
        Some(deps) if !deps.is_empty() => json!(deps),
        _ => json!(null),
    };
    json!({
        "ticket_id": report.ticket_id,
        "path": report.path.display().to_string(),
        "metadata": {
            "name": report.name,
            "creation_date": report.creation_date,
            "status": report.status,
            "depends_on": depends_on,
        },
        "file_size": report.file_size,
    })
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use toml::Value;

    use super::TicketMetadata;

    #[test]
    fn ticket_metadata_depends_on_round_trips() {
        let path = Path::new("ticket.md");
        let toml = "title = \"Test\"\ncreation_date = 2026-06-22T12:00:00Z\nstatus = \"pending\"\ndepends_on = [\"tt-dep-a\", \"tt-dep-b\"]\n";
        let value: Value = toml.parse().unwrap();

        let metadata = TicketMetadata::from_frontmatter(&value, path, "tt-test").unwrap();
        assert_eq!(metadata.ticket_id, "tt-test");
        assert_eq!(metadata.name.as_deref(), Some("Test"));
        assert_eq!(
            metadata.depends_on,
            Some(vec!["tt-dep-a".to_string(), "tt-dep-b".to_string()])
        );

        let lines = metadata.to_frontmatter_lines();
        assert!(lines.starts_with("name = \"Test\"\n"));
        assert!(!lines.contains("ticket_id"));
        assert!(!lines.contains("title ="));
        assert!(lines.contains("depends_on = [\"tt-dep-a\", \"tt-dep-b\"]"));
    }

    #[test]
    fn ticket_metadata_reads_new_name_field() {
        let path = Path::new("ticket.md");
        let toml = "name = \"Test\"\ncreation_date = 2026-06-22T12:00:00Z\nstatus = \"pending\"\n";
        let value: Value = toml.parse().unwrap();

        let metadata = TicketMetadata::from_frontmatter(&value, path, "tt-test").unwrap();

        assert_eq!(metadata.name.as_deref(), Some("Test"));
        assert!(metadata
            .to_frontmatter_lines()
            .starts_with("name = \"Test\"\n"));
    }

    #[test]
    fn ticket_metadata_missing_depends_on_is_none() {
        let path = Path::new("ticket.md");
        let toml = "title = \"Test\"\ncreation_date = 2026-06-22T12:00:00Z\nstatus = \"pending\"\n";
        let value: Value = toml.parse().unwrap();

        let metadata = TicketMetadata::from_frontmatter(&value, path, "tt-test").unwrap();
        assert_eq!(metadata.depends_on, None);
        assert!(!metadata.to_frontmatter_lines().contains("depends_on"));
    }

    #[test]
    fn ticket_metadata_empty_depends_on_is_none() {
        let path = Path::new("ticket.md");
        let toml = "title = \"Test\"\ncreation_date = 2026-06-22T12:00:00Z\nstatus = \"pending\"\ndepends_on = []\n";
        let value: Value = toml.parse().unwrap();

        let metadata = TicketMetadata::from_frontmatter(&value, path, "tt-test").unwrap();
        assert_eq!(metadata.depends_on, None);
        assert!(!metadata.to_frontmatter_lines().contains("depends_on"));
    }

    #[test]
    fn ticket_metadata_depends_on_non_array_is_error() {
        let path = Path::new("ticket.md");
        let toml = "title = \"Test\"\ncreation_date = 2026-06-22T12:00:00Z\nstatus = \"pending\"\ndepends_on = \"tt-dep-a\"\n";
        let value: Value = toml.parse().unwrap();

        let errors = TicketMetadata::from_frontmatter(&value, path, "tt-test")
            .err()
            .unwrap();
        assert!(errors
            .iter()
            .any(|e| e.contains("depends_on") && e.contains("array")));
    }

    #[test]
    fn ticket_metadata_unknown_field_is_error() {
        let path = Path::new("ticket.md");
        let toml = "title = \"Test\"\ncreation_date = 2026-06-22T12:00:00Z\nstatus = \"pending\"\ndependencies = [\"tt-dep-a\"]\n";
        let value: Value = toml.parse().unwrap();

        let errors = TicketMetadata::from_frontmatter(&value, path, "tt-test")
            .err()
            .unwrap();
        assert!(errors
            .iter()
            .any(|e| e.contains("unknown field dependencies")));
    }
}
