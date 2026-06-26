use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use clap::ValueEnum;
use serde_json::json;
use toml::Value;

use crate::frontmatter::{
    datetime_string, invalid_frontmatter_error, parse_frontmatter, parse_frontmatter_from_contents,
    require_datetime, require_optional_string_array, require_string, require_string_choice,
    serialize_record,
};
use crate::ids::{random_hex_chars, toml_string};
use crate::record::{markdown_body_after_frontmatter, WaapRecordKind};

pub(crate) mod get;
pub(crate) mod list;
pub(crate) mod new;
pub(crate) mod update;

pub(crate) use get::{get_ticket, print_ticket_get_report};
pub(crate) use list::{list_tickets, print_ticket_list};
pub(crate) use new::{create_ticket, print_ticket_report};
pub(crate) use update::{print_updated_ticket_report, update_ticket_status};

pub(crate) struct TicketMetadata {
    pub(crate) title: String,
    pub(crate) creation_date: String,
    pub(crate) status: String,
    pub(crate) depends_on: Option<Vec<String>>,
}

impl TicketMetadata {
    pub(crate) fn from_frontmatter(value: &Value, path: &Path) -> Result<Self, Vec<String>> {
        let mut errors = Vec::new();
        require_string(value, "title", path, &mut errors);
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
            title: value
                .get("title")
                .and_then(Value::as_str)
                .expect("validated title")
                .to_string(),
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
        lines.push_str(&format!("title = {}\n", toml_string(&self.title)));
        lines.push_str(&format!("creation_date = {}\n", self.creation_date));
        lines.push_str(&format!("status = {}\n", toml_string(&self.status)));
        if let Some(deps) = &self.depends_on {
            if !deps.is_empty() {
                let items: Vec<String> = deps.iter().map(|d| toml_string(d)).collect();
                lines.push_str(&format!("depends_on = [{}]\n", items.join(", ")));
            }
        }
        lines
    }
}

pub(crate) fn ticket_path(repo_root: &Path, ticket_id: &str) -> PathBuf {
    WaapRecordKind::Ticket
        .root_path(repo_root)
        .join(ticket_id)
        .join("ticket.md")
}

pub(crate) fn load_ticket_metadata(
    repo_root: &Path,
    ticket_id: &str,
) -> io::Result<TicketMetadata> {
    let path = validate_ticket_path(repo_root, ticket_id)?;
    let contents = fs::read_to_string(&path)?;
    let mut errors = Vec::new();
    let Some(value) = parse_frontmatter_from_contents(&contents, &path, &mut errors) else {
        return Err(invalid_frontmatter_error(errors));
    };
    TicketMetadata::from_frontmatter(&value, &path).map_err(invalid_frontmatter_error)
}

pub(crate) fn read_ticket_record(
    repo_root: &Path,
    ticket_id: &str,
) -> io::Result<(TicketMetadata, String)> {
    let path = validate_ticket_path(repo_root, ticket_id)?;
    let contents = fs::read_to_string(&path)?;
    let mut errors = Vec::new();
    let Some(value) = parse_frontmatter_from_contents(&contents, &path, &mut errors) else {
        return Err(invalid_frontmatter_error(errors));
    };
    let metadata =
        TicketMetadata::from_frontmatter(&value, &path).map_err(invalid_frontmatter_error)?;
    let body = markdown_body_after_frontmatter(&contents)?;
    Ok((metadata, body))
}

fn validate_ticket_path(repo_root: &Path, ticket_id: &str) -> io::Result<PathBuf> {
    if !is_ticket_id(ticket_id) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("{ticket_id:?} is not a valid ticket id"),
        ));
    }
    let path = ticket_path(repo_root, ticket_id);
    if !path.is_file() {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            format!("{} does not exist", path.display()),
        ));
    }
    Ok(path)
}

pub(crate) fn write_ticket_record(
    repo_root: &Path,
    ticket_id: &str,
    metadata: &TicketMetadata,
    body: &str,
) -> io::Result<()> {
    let path = ticket_path(repo_root, ticket_id);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let contents = serialize_record(&metadata.to_frontmatter_lines(), body);
    fs::write(path, contents)
}

pub(crate) fn check_ticket_frontmatter(path: &Path, errors: &mut Vec<String>) {
    let Some(frontmatter) = parse_frontmatter(path, errors) else {
        return;
    };
    if let Err(mut frontmatter_errors) = TicketMetadata::from_frontmatter(&frontmatter, path) {
        errors.append(&mut frontmatter_errors);
    }
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct TicketReport {
    pub(crate) ticket_id: String,
    pub(crate) path: PathBuf,
    pub(crate) title: String,
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

pub(crate) fn available_ticket_id(tickets_dir: &Path, title: &str) -> io::Result<String> {
    let mut ticket_id = format!("tt-{}", slugify_title(title)?);
    if !tickets_dir.join(&ticket_id).exists() {
        return Ok(ticket_id);
    }

    loop {
        ticket_id = format!("tt-{}", slug_with_hash(title)?);
        if !tickets_dir.join(&ticket_id).exists() {
            return Ok(ticket_id);
        }
    }
}

pub(crate) fn slugify_title(title: &str) -> io::Result<String> {
    let mut slug = String::new();
    let mut previous_dash = false;

    for byte in title.trim().bytes() {
        match byte {
            b'A'..=b'Z' => {
                slug.push((byte + 32) as char);
                previous_dash = false;
            }
            b'a'..=b'z' | b'0'..=b'9' => {
                slug.push(byte as char);
                previous_dash = false;
            }
            b' ' | b'\t' | b'\n' | b'\r' | b'-' if !slug.is_empty() && !previous_dash => {
                slug.push('-');
                previous_dash = true;
            }
            _ => {}
        }
    }

    while slug.ends_with('-') {
        slug.pop();
    }

    if slug.is_empty() {
        slug.push_str("ticket");
    }

    if slug.len() > 63 {
        slug = slug_with_hash_from_slug(&slug, &random_hex_chars(4)?);
    }

    Ok(slug)
}

pub(crate) fn slug_with_hash(title: &str) -> io::Result<String> {
    Ok(slug_with_hash_from_slug(
        &slugify_title(title)?,
        &random_hex_chars(4)?,
    ))
}

pub(crate) fn slug_with_hash_from_slug(slug: &str, hash: &str) -> String {
    let max_prefix_len = 58;
    let prefix_len = slug.len().min(max_prefix_len);
    let mut prefix = slug[..prefix_len].trim_end_matches('-').to_string();
    if prefix.is_empty() {
        prefix.push_str("ticket");
    }
    format!("{prefix}-{hash}")
}

pub(crate) fn is_ticket_id(value: &str) -> bool {
    let Some(slug) = value.strip_prefix("tt-") else {
        return false;
    };

    !slug.is_empty()
        && slug.len() < 64
        && !slug.starts_with('-')
        && !slug.ends_with('-')
        && !slug.contains("--")
        && slug
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
}

pub(crate) fn print_ticket_report_human(header: &str, report: &TicketReport) {
    println!("{header} {}", report.ticket_id);
    println!("Path: {}", report.path.display());
    println!("Title: {}", report.title);
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
            "title": report.title,
            "creation_date": report.creation_date,
            "status": report.status,
            "depends_on": depends_on,
        },
        "file_size": report.file_size,
    })
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::Path;

    use tempfile::tempdir;
    use toml::Value;

    use super::{available_ticket_id, slugify_title, TicketMetadata};

    #[test]
    fn slug_generation_matches_spec_rules() {
        assert_eq!(
            slugify_title("  List All Tickets!  ").unwrap(),
            "list-all-tickets"
        );
        assert_eq!(
            slugify_title("Bad---Spaces   Here").unwrap(),
            "bad-spaces-here"
        );
        assert_eq!(slugify_title("Café: déjà vu").unwrap(), "caf-dj-vu");
    }

    #[test]
    fn long_slug_is_truncated_with_hex_hash() {
        let slug = slugify_title(
            "This is a very long ticket title that should be truncated because it exceeds limits",
        )
        .unwrap();

        assert!(slug.len() <= 63);
        assert_eq!(slug.as_bytes()[slug.len() - 5], b'-');
        assert!(slug[slug.len() - 4..]
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase()));
    }

    #[test]
    fn ticket_metadata_depends_on_round_trips() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("ticket.md");
        let toml = "title = \"Test\"\ncreation_date = 2026-06-22T12:00:00Z\nstatus = \"pending\"\ndepends_on = [\"tt-dep-a\", \"tt-dep-b\"]\n";
        let value: Value = toml.parse().unwrap();

        let metadata = TicketMetadata::from_frontmatter(&value, &path).unwrap();
        assert_eq!(
            metadata.depends_on,
            Some(vec!["tt-dep-a".to_string(), "tt-dep-b".to_string()])
        );

        let lines = metadata.to_frontmatter_lines();
        assert!(lines.contains("depends_on = [\"tt-dep-a\", \"tt-dep-b\"]"));
    }

    #[test]
    fn ticket_metadata_missing_depends_on_is_none() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("ticket.md");
        let toml = "title = \"Test\"\ncreation_date = 2026-06-22T12:00:00Z\nstatus = \"pending\"\n";
        let value: Value = toml.parse().unwrap();

        let metadata = TicketMetadata::from_frontmatter(&value, &path).unwrap();
        assert_eq!(metadata.depends_on, None);
        assert!(!metadata.to_frontmatter_lines().contains("depends_on"));
    }

    #[test]
    fn ticket_metadata_empty_depends_on_is_none() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("ticket.md");
        let toml = "title = \"Test\"\ncreation_date = 2026-06-22T12:00:00Z\nstatus = \"pending\"\ndepends_on = []\n";
        let value: Value = toml.parse().unwrap();

        let metadata = TicketMetadata::from_frontmatter(&value, &path).unwrap();
        assert_eq!(metadata.depends_on, None);
        assert!(!metadata.to_frontmatter_lines().contains("depends_on"));
    }

    #[test]
    fn ticket_metadata_depends_on_non_array_is_error() {
        let path = Path::new("ticket.md");
        let toml = "title = \"Test\"\ncreation_date = 2026-06-22T12:00:00Z\nstatus = \"pending\"\ndepends_on = \"tt-dep-a\"\n";
        let value: Value = toml.parse().unwrap();

        let errors = TicketMetadata::from_frontmatter(&value, path)
            .err()
            .unwrap();
        assert!(errors
            .iter()
            .any(|e| e.contains("depends_on") && e.contains("array")));
    }

    #[test]
    fn conflict_handling_appends_hex_hash() {
        let dir = tempdir().unwrap();
        fs::create_dir_all(dir.path().join(".waap/tickets/tt-list-tickets")).unwrap();

        let ticket_id =
            available_ticket_id(&dir.path().join(".waap/tickets"), "List Tickets").unwrap();

        assert_ne!(ticket_id, "tt-list-tickets");
        assert!(ticket_id.starts_with("tt-list-tickets-"));
        assert_eq!(ticket_id.len(), "tt-list-tickets-0000".len());
        assert!(ticket_id[ticket_id.len() - 4..]
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase()));
    }
}
