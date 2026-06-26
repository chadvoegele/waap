use std::io;
use std::path::Path;

use serde_json::json;

use crate::cli::OutputFormat;
use crate::record::{list_record_ids, WaapRecordKind};
use crate::ticket::get::load_ticket_report;
use crate::ticket::{TicketReport, TicketStatus};

pub(crate) fn print_ticket_list(output_format: &OutputFormat, reports: &[TicketReport]) {
    let ticket_ids: Vec<&str> = reports
        .iter()
        .map(|report| report.ticket_id.as_str())
        .collect();
    match output_format {
        OutputFormat::Json => println!("{}", ticket_list_json(reports)),
        OutputFormat::HumanReadable => {
            for ticket_id in ticket_ids {
                println!("{ticket_id}");
            }
        }
    }
}

pub(crate) fn ticket_list_json(reports: &[TicketReport]) -> serde_json::Value {
    json!(reports
        .iter()
        .map(|report| report.ticket_id.as_str())
        .collect::<Vec<_>>())
}

pub(crate) fn list_tickets(
    repo_root: &Path,
    status: Option<&TicketStatus>,
) -> io::Result<Vec<TicketReport>> {
    let mut reports = Vec::new();
    for ticket_id in list_record_ids(repo_root, WaapRecordKind::Ticket)? {
        let report = load_ticket_report(repo_root, &ticket_id)?;
        if status.is_none_or(|status| report.status == status.as_str()) {
            reports.push(report);
        }
    }

    Ok(reports)
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::io;
    use std::path::{Path, PathBuf};

    use serde_json::json;
    use tempfile::tempdir;

    use super::{list_tickets, ticket_list_json};
    use crate::ticket::TicketReport;
    use crate::ticket::TicketStatus;

    #[test]
    fn ticket_list_returns_sorted_ticket_ids() {
        let dir = tempdir().unwrap();
        write_ticket(dir.path(), "tt-z-ticket", "completed");
        write_ticket(dir.path(), "tt-a-ticket", "pending");
        write_ticket(dir.path(), "tt-m-ticket", "in-progress");

        let reports = list_tickets(dir.path(), None).unwrap();

        assert_eq!(
            ticket_ids(&reports),
            vec!["tt-a-ticket", "tt-m-ticket", "tt-z-ticket"]
        );
    }

    #[test]
    fn ticket_list_filters_by_status() {
        let dir = tempdir().unwrap();
        write_ticket(dir.path(), "tt-one", "pending");
        write_ticket(dir.path(), "tt-two", "completed");
        write_ticket(dir.path(), "tt-three", "completed");

        let reports = list_tickets(dir.path(), Some(&TicketStatus::Completed)).unwrap();

        assert_eq!(ticket_ids(&reports), vec!["tt-three", "tt-two"]);
    }

    #[test]
    fn ticket_list_handles_empty_ticket_directories() {
        let dir = tempdir().unwrap();
        fs::create_dir_all(dir.path().join(".waap/tickets")).unwrap();

        let reports = list_tickets(dir.path(), None).unwrap();

        assert!(reports.is_empty());
    }

    #[test]
    fn ticket_list_handles_missing_ticket_directories() {
        let dir = tempdir().unwrap();

        let reports = list_tickets(dir.path(), None).unwrap();

        assert!(reports.is_empty());
    }

    #[test]
    fn ticket_list_rejects_non_directory_entries() {
        let dir = tempdir().unwrap();
        write_file(&dir.path().join(".waap/tickets/not-a-directory"), "oops");

        let error = list_tickets(dir.path(), None).unwrap_err();

        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
        assert!(error.to_string().contains("must be a ticket directory"));
    }

    #[test]
    fn ticket_list_validates_ticket_frontmatter() {
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

        let error = list_tickets(dir.path(), None).unwrap_err();

        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
        assert!(error.to_string().contains("status has invalid value"));
    }

    #[test]
    fn ticket_list_json_has_expected_shape() {
        let reports = vec![
            TicketReport {
                ticket_id: "tt-one".to_string(),
                path: PathBuf::from(".waap/tickets/tt-one/ticket.md"),
                title: "One".to_string(),
                creation_date: "2026-06-22T12:00:00Z".to_string(),
                status: "pending".to_string(),
                depends_on: None,
                file_size: 123,
            },
            TicketReport {
                ticket_id: "tt-two".to_string(),
                path: PathBuf::from(".waap/tickets/tt-two/ticket.md"),
                title: "Two".to_string(),
                creation_date: "2026-06-22T12:00:00Z".to_string(),
                status: "completed".to_string(),
                depends_on: None,
                file_size: 456,
            },
        ];

        assert_eq!(ticket_list_json(&reports), json!(["tt-one", "tt-two"]));
    }

    fn ticket_ids(reports: &[TicketReport]) -> Vec<&str> {
        reports
            .iter()
            .map(|report| report.ticket_id.as_str())
            .collect()
    }

    fn write_ticket(repo_root: &Path, ticket_id: &str, status: &str) {
        write_file(
            &repo_root.join(format!(".waap/tickets/{ticket_id}/ticket.md")),
            &format!(
                "+++\ntitle = \"Test Ticket\"\ncreation_date = 2026-06-18T15:00:34Z\nstatus = \"{status}\"\n+++\n\n# Description\n"
            ),
        );
    }

    fn write_file(path: &Path, contents: &str) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, contents).unwrap();
    }
}
