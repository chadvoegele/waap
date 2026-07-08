use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use crate::agent::is_agent_id;
use crate::ticket::is_ticket_id;

#[derive(Clone, Copy, Debug)]
pub(crate) enum WaapRecordKind {
    Agent,
    Ticket,
}

impl WaapRecordKind {
    pub(crate) fn directory_name(self) -> &'static str {
        match self {
            WaapRecordKind::Agent => "agents",
            WaapRecordKind::Ticket => "tickets",
        }
    }

    pub(crate) fn singular_name(self) -> &'static str {
        match self {
            WaapRecordKind::Agent => "agent",
            WaapRecordKind::Ticket => "ticket",
        }
    }

    pub(crate) fn directory_description(self) -> &'static str {
        match self {
            WaapRecordKind::Agent => "an agent directory",
            WaapRecordKind::Ticket => "a ticket directory",
        }
    }

    pub(crate) fn id_example(self) -> &'static str {
        match self {
            WaapRecordKind::Agent => "aa-3881fda0",
            WaapRecordKind::Ticket => "tt-list-tickets",
        }
    }

    pub(crate) fn is_valid_id(self, value: &str) -> bool {
        match self {
            WaapRecordKind::Agent => is_agent_id(value),
            WaapRecordKind::Ticket => is_ticket_id(value),
        }
    }

    pub(crate) fn root_label(self) -> String {
        format!(".waap/{}", self.directory_name())
    }

    pub(crate) fn root_path(self, waap_root: &Path) -> PathBuf {
        waap_root.join(".waap").join(self.directory_name())
    }
}

pub(crate) fn list_record_ids(waap_root: &Path, kind: WaapRecordKind) -> io::Result<Vec<String>> {
    let records_dir = kind.root_path(waap_root);
    let records_label = kind.root_label();
    if !records_dir.exists() {
        return Ok(Vec::new());
    }
    if !records_dir.is_dir() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("{records_label} must be a directory"),
        ));
    }

    let mut ids = Vec::new();
    for entry in fs::read_dir(&records_dir)? {
        let entry = entry?;
        let path = entry.path();
        let id = entry.file_name().to_string_lossy().into_owned();
        let label = format!("{records_label}/{id}");

        if !path.is_dir() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("{label} must be {}", kind.directory_description()),
            ));
        }
        if !kind.is_valid_id(&id) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!(
                    "{label} must be named as a {} id like {}",
                    kind.singular_name(),
                    kind.id_example()
                ),
            ));
        }

        ids.push(id);
    }
    ids.sort();

    Ok(ids)
}

pub(crate) fn markdown_body_after_frontmatter(contents: &str) -> io::Result<String> {
    let mut delimiter_count = 0;
    let mut offset = 0;

    for line in contents.split_inclusive('\n') {
        let line_without_newline = line.trim_end_matches('\n').trim_end_matches('\r');
        if line_without_newline == "+++" {
            delimiter_count += 1;
            if delimiter_count == 2 {
                return Ok(contents[offset + line.len()..].to_string());
            }
        }
        offset += line.len();
    }

    if contents[offset..].trim_end_matches('\r') == "+++" {
        return Ok(String::new());
    }

    Err(io::Error::new(
        io::ErrorKind::InvalidData,
        "frontmatter is missing closing +++ delimiter",
    ))
}
