use std::fs;
use std::io;
use std::path::Path;

use serde_json::json;

use crate::cli::OutputFormat;
use crate::ticket::{
    load_ticket_metadata, print_ticket_report_human, read_ticket_record, ticket_path,
    ticket_report_json, TicketGetReport, TicketReport,
};

pub(crate) fn print_ticket_get_report(output_format: &OutputFormat, report: &TicketGetReport) {
    match output_format {
        OutputFormat::Json => println!("{}", ticket_get_report_json(report)),
        OutputFormat::HumanReadable => {
            print_ticket_report_human("Ticket", &report.ticket);
            println!("Content:");
            print!("{}", report.content);
        }
    }
}

pub(crate) fn ticket_get_report_json(report: &TicketGetReport) -> serde_json::Value {
    let mut value = ticket_report_json(&report.ticket);
    value["content"] = json!(report.content);
    value
}

pub(crate) fn load_ticket_report(waap_root: &Path, ticket_id: &str) -> io::Result<TicketReport> {
    let path = ticket_path(waap_root, ticket_id);
    let metadata = load_ticket_metadata(waap_root, ticket_id)?;

    Ok(TicketReport {
        ticket_id: ticket_id.to_string(),
        path: path.clone(),
        name: metadata.name,
        creation_date: metadata.creation_date,
        status: metadata.status,
        depends_on: metadata.depends_on,
        file_size: fs::metadata(&path)?.len(),
    })
}

pub(crate) fn get_ticket(waap_root: &Path, ticket_id: &str) -> io::Result<TicketGetReport> {
    let ticket = load_ticket_report(waap_root, ticket_id)?;
    let (_, body) = read_ticket_record(waap_root, ticket_id)?;
    let content = body.strip_prefix('\n').unwrap_or(&body).to_string();

    Ok(TicketGetReport { ticket, content })
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::io;
    use std::path::{Path, PathBuf};

    use serde_json::json;
    use tempfile::tempdir;

    use super::{get_ticket, ticket_get_report_json};
    use crate::ticket::{TicketGetReport, TicketReport};

    #[test]
    fn ticket_get_reads_metadata_and_markdown_content() {
        let dir = tempdir().unwrap();
        let contents = "+++\ntitle = \"New Ticket\"\ncreation_date = 2026-06-22T12:00:00Z\nstatus = \"pending\"\n+++\n\n# Description\nKeep this body exactly.\n";
        write_file(
            &dir.path().join(".waap/tickets/tt-new-ticket/ticket.md"),
            contents,
        );

        let report = get_ticket(dir.path(), "tt-new-ticket").unwrap();

        assert_eq!(report.ticket.ticket_id, "tt-new-ticket");
        assert_eq!(report.ticket.name.as_deref(), Some("New Ticket"));
        assert_eq!(report.ticket.creation_date, "2026-06-22T12:00:00Z");
        assert_eq!(report.ticket.status, "pending");
        assert_eq!(report.ticket.file_size, contents.len() as u64);
        assert_eq!(report.content, "# Description\nKeep this body exactly.\n");
    }

    #[test]
    fn ticket_get_reports_missing_ticket() {
        let dir = tempdir().unwrap();

        let error = get_ticket(dir.path(), "tt-new-ticket").unwrap_err();

        assert_eq!(error.kind(), io::ErrorKind::NotFound);
        assert!(error
            .to_string()
            .contains(".waap/tickets/tt-new-ticket/ticket.md"));
    }

    #[test]
    fn ticket_get_rejects_invalid_ticket_id() {
        let dir = tempdir().unwrap();

        let error = get_ticket(dir.path(), "new-ticket").unwrap_err();

        assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
        assert!(error.to_string().contains("not a valid ticket id"));
    }

    #[test]
    fn ticket_get_validates_ticket_frontmatter() {
        let dir = tempdir().unwrap();
        write_file(
            &dir.path().join(".waap/tickets/tt-bad-ticket/ticket.md"),
            "+++
title = \"Bad Ticket\"
creation_date = 2026-06-18T15:00:34Z
status = \"ready\"
+++
",
        );

        let error = get_ticket(dir.path(), "tt-bad-ticket").unwrap_err();

        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
        assert!(error.to_string().contains("status has invalid value"));
    }

    #[test]
    fn ticket_get_json_has_expected_shape() {
        let report = TicketGetReport {
            ticket: TicketReport {
                ticket_id: "tt-new-ticket".to_string(),
                path: PathBuf::from(".waap/tickets/tt-new-ticket/ticket.md"),
                name: Some("New Ticket".to_string()),
                creation_date: "2026-06-22T12:00:00Z".to_string(),
                status: "pending".to_string(),
                depends_on: None,
                file_size: 123,
            },
            content: "# Body\n".to_string(),
        };

        assert_eq!(
            ticket_get_report_json(&report),
            json!({
                "ticket_id": "tt-new-ticket",
                "path": ".waap/tickets/tt-new-ticket/ticket.md",
                "metadata": {
                    "name": "New Ticket",
                    "creation_date": "2026-06-22T12:00:00Z",
                    "status": "pending",
                    "depends_on": null,
                },
                "file_size": 123,
                "content": "# Body\n",
            })
        );
    }

    #[test]
    fn ticket_get_json_includes_depends_on_when_present() {
        let report = TicketGetReport {
            ticket: TicketReport {
                ticket_id: "tt-feature".to_string(),
                path: PathBuf::from(".waap/tickets/tt-feature/ticket.md"),
                name: Some("Feature".to_string()),
                creation_date: "2026-06-22T12:00:00Z".to_string(),
                status: "pending".to_string(),
                depends_on: Some(vec!["tt-dep-one".to_string(), "tt-dep-two".to_string()]),
                file_size: 200,
            },
            content: "# Body\n".to_string(),
        };

        let json = ticket_get_report_json(&report);
        assert_eq!(
            json["metadata"]["depends_on"],
            json!(["tt-dep-one", "tt-dep-two"])
        );
    }

    #[test]
    fn ticket_get_reads_depends_on_from_frontmatter() {
        let dir = tempdir().unwrap();
        let contents = "+++\ntitle = \"Feature\"\ncreation_date = 2026-06-22T12:00:00Z\nstatus = \"pending\"\ndepends_on = [\"tt-dep-one\", \"tt-dep-two\"]\n+++\n\n# Body\n";
        write_file(
            &dir.path().join(".waap/tickets/tt-feature/ticket.md"),
            contents,
        );

        let report = get_ticket(dir.path(), "tt-feature").unwrap();

        assert_eq!(
            report.ticket.depends_on,
            Some(vec!["tt-dep-one".to_string(), "tt-dep-two".to_string()])
        );
    }

    fn write_file(path: &Path, contents: &str) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, contents).unwrap();
    }
}
