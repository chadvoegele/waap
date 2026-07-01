use std::collections::HashMap;
use std::io;
use std::path::Path;

use serde_json::json;

use crate::cli::OutputFormat;
use crate::record::{list_record_ids, WaapRecordKind};
use crate::ticket::get::load_ticket_report;
use crate::ticket::{TicketReport, TicketStatus};

#[derive(Debug)]
pub(crate) struct TicketListEntry {
    pub(crate) report: TicketReport,
    pub(crate) blocked: bool,
}

fn compute_blocked(report: &TicketReport, status_map: &HashMap<String, String>) -> bool {
    match &report.depends_on {
        None => false,
        Some(deps) => deps
            .iter()
            .any(|dep_id| status_map.get(dep_id).is_none_or(|s| s != "completed")),
    }
}

pub(crate) fn print_ticket_list(output_format: &OutputFormat, entries: &[TicketListEntry]) {
    match output_format {
        OutputFormat::Json => println!("{}", ticket_list_json(entries)),
        OutputFormat::HumanReadable => {
            for line in ticket_list_human_lines(entries) {
                println!("{line}");
            }
        }
    }
}

const TICKET_ID_HEADER: &str = "TICKET ID";
const STATUS_HEADER: &str = "STATUS";
const STATE_HEADER: &str = "STATE";

fn ticket_list_human_lines(entries: &[TicketListEntry]) -> Vec<String> {
    if entries.is_empty() {
        return Vec::new();
    }

    let has_state_column = entries
        .iter()
        .any(|entry| entry.report.depends_on.is_some());
    let id_width = entries
        .iter()
        .map(|entry| entry.report.ticket_id.len())
        .max()
        .unwrap_or(0)
        .max(TICKET_ID_HEADER.len());

    let header = if has_state_column {
        format!("{TICKET_ID_HEADER:id_width$}  {STATUS_HEADER}  {STATE_HEADER}")
    } else {
        format!("{TICKET_ID_HEADER:id_width$}  {STATUS_HEADER}")
    };

    let mut lines = vec![header];
    lines.extend(entries.iter().map(|entry| {
        let id = &entry.report.ticket_id;
        let status = &entry.report.status;
        if entry.report.depends_on.is_some() {
            let state = if entry.blocked {
                "[blocked]"
            } else {
                "[unblocked]"
            };
            format!("{id:id_width$}  {status}  {state}")
        } else {
            format!("{id:id_width$}  {status}")
        }
    }));
    lines
}

pub(crate) fn ticket_list_json(entries: &[TicketListEntry]) -> serde_json::Value {
    json!(entries
        .iter()
        .map(|entry| json!({"ticket_id": entry.report.ticket_id, "status": entry.report.status, "blocked": entry.blocked}))
        .collect::<Vec<_>>())
}

pub(crate) fn list_tickets(
    waap_root: &Path,
    status: Option<&TicketStatus>,
    blocked_filter: Option<bool>,
) -> io::Result<Vec<TicketListEntry>> {
    let all_ids = list_record_ids(waap_root, WaapRecordKind::Ticket)?;

    let mut all_reports: Vec<TicketReport> = Vec::new();
    for ticket_id in &all_ids {
        all_reports.push(load_ticket_report(waap_root, ticket_id)?);
    }

    let status_map: HashMap<String, String> = all_reports
        .iter()
        .map(|r| (r.ticket_id.clone(), r.status.clone()))
        .collect();

    let mut entries = Vec::new();
    for report in all_reports {
        let blocked = compute_blocked(&report, &status_map);
        if status.is_none_or(|s| report.status == s.as_str())
            && blocked_filter.is_none_or(|filter| blocked == filter)
        {
            entries.push(TicketListEntry { report, blocked });
        }
    }

    entries.sort_by(|a, b| a.report.creation_date.cmp(&b.report.creation_date));

    Ok(entries)
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::io;
    use std::path::{Path, PathBuf};

    use serde_json::json;
    use tempfile::tempdir;

    use super::{list_tickets, ticket_list_human_lines, ticket_list_json, TicketListEntry};
    use crate::ticket::TicketReport;
    use crate::ticket::TicketStatus;

    #[test]
    fn ticket_list_returns_sorted_ticket_ids() {
        let dir = tempdir().unwrap();
        write_ticket(dir.path(), "tt-z-ticket", "completed", &[]);
        write_ticket(dir.path(), "tt-a-ticket", "pending", &[]);
        write_ticket(dir.path(), "tt-m-ticket", "in-progress", &[]);

        let entries = list_tickets(dir.path(), None, None).unwrap();

        assert_eq!(
            ticket_ids(&entries),
            vec!["tt-a-ticket", "tt-m-ticket", "tt-z-ticket"]
        );
    }

    #[test]
    fn ticket_list_orders_by_creation_date_not_id() {
        let dir = tempdir().unwrap();
        write_ticket_with_creation_date(
            dir.path(),
            "tt-a-ticket",
            "pending",
            "2026-06-20T00:00:00Z",
        );
        write_ticket_with_creation_date(
            dir.path(),
            "tt-z-ticket",
            "pending",
            "2026-06-18T00:00:00Z",
        );
        write_ticket_with_creation_date(
            dir.path(),
            "tt-m-ticket",
            "pending",
            "2026-06-19T00:00:00Z",
        );

        let entries = list_tickets(dir.path(), None, None).unwrap();

        assert_eq!(
            ticket_ids(&entries),
            vec!["tt-z-ticket", "tt-m-ticket", "tt-a-ticket"]
        );
    }

    #[test]
    fn ticket_list_filters_by_status() {
        let dir = tempdir().unwrap();
        write_ticket(dir.path(), "tt-one", "pending", &[]);
        write_ticket(dir.path(), "tt-two", "completed", &[]);
        write_ticket(dir.path(), "tt-three", "completed", &[]);

        let entries = list_tickets(dir.path(), Some(&TicketStatus::Completed), None).unwrap();

        assert_eq!(ticket_ids(&entries), vec!["tt-three", "tt-two"]);
    }

    #[test]
    fn ticket_list_handles_empty_ticket_directories() {
        let dir = tempdir().unwrap();
        fs::create_dir_all(dir.path().join(".waap/tickets")).unwrap();

        let entries = list_tickets(dir.path(), None, None).unwrap();

        assert!(entries.is_empty());
    }

    #[test]
    fn ticket_list_handles_missing_ticket_directories() {
        let dir = tempdir().unwrap();

        let entries = list_tickets(dir.path(), None, None).unwrap();

        assert!(entries.is_empty());
    }

    #[test]
    fn ticket_list_rejects_non_directory_entries() {
        let dir = tempdir().unwrap();
        write_file(&dir.path().join(".waap/tickets/not-a-directory"), "oops");

        let error = list_tickets(dir.path(), None, None).unwrap_err();

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

        let error = list_tickets(dir.path(), None, None).unwrap_err();

        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
        assert!(error.to_string().contains("status has invalid value"));
    }

    #[test]
    fn ticket_list_json_has_expected_shape() {
        let entries = vec![
            TicketListEntry {
                report: TicketReport {
                    ticket_id: "tt-one".to_string(),
                    path: PathBuf::from(".waap/tickets/tt-one/ticket.md"),
                    title: "One".to_string(),
                    creation_date: "2026-06-22T12:00:00Z".to_string(),
                    status: "pending".to_string(),
                    depends_on: None,
                    file_size: 123,
                },
                blocked: false,
            },
            TicketListEntry {
                report: TicketReport {
                    ticket_id: "tt-two".to_string(),
                    path: PathBuf::from(".waap/tickets/tt-two/ticket.md"),
                    title: "Two".to_string(),
                    creation_date: "2026-06-22T12:00:00Z".to_string(),
                    status: "completed".to_string(),
                    depends_on: None,
                    file_size: 456,
                },
                blocked: false,
            },
        ];

        assert_eq!(
            ticket_list_json(&entries),
            json!([
                {"ticket_id": "tt-one", "status": "pending", "blocked": false},
                {"ticket_id": "tt-two", "status": "completed", "blocked": false},
            ])
        );
    }

    #[test]
    fn ticket_list_json_includes_status_field() {
        let entries = vec![TicketListEntry {
            report: TicketReport {
                ticket_id: "tt-one".to_string(),
                path: PathBuf::from(".waap/tickets/tt-one/ticket.md"),
                title: "One".to_string(),
                creation_date: "2026-06-22T12:00:00Z".to_string(),
                status: "in-progress".to_string(),
                depends_on: None,
                file_size: 123,
            },
            blocked: false,
        }];

        let value = ticket_list_json(&entries);
        assert_eq!(value[0]["status"], json!("in-progress"));
    }

    #[test]
    fn ticket_list_human_lines_show_status() {
        let entries = vec![
            TicketListEntry {
                report: TicketReport {
                    ticket_id: "tt-one".to_string(),
                    path: PathBuf::from(".waap/tickets/tt-one/ticket.md"),
                    title: "One".to_string(),
                    creation_date: "2026-06-22T12:00:00Z".to_string(),
                    status: "completed".to_string(),
                    depends_on: None,
                    file_size: 123,
                },
                blocked: false,
            },
            TicketListEntry {
                report: TicketReport {
                    ticket_id: "tt-feature".to_string(),
                    path: PathBuf::from(".waap/tickets/tt-feature/ticket.md"),
                    title: "Feature".to_string(),
                    creation_date: "2026-06-22T12:00:00Z".to_string(),
                    status: "pending".to_string(),
                    depends_on: Some(vec!["tt-one".to_string()]),
                    file_size: 456,
                },
                blocked: true,
            },
        ];

        let lines = ticket_list_human_lines(&entries);

        assert_eq!(lines[0], "TICKET ID   STATUS  STATE");
        assert_eq!(lines[1], "tt-one      completed");
        assert_eq!(lines[2], "tt-feature  pending  [blocked]");
    }

    #[test]
    fn ticket_list_human_lines_header_without_state_column() {
        let entries = vec![TicketListEntry {
            report: TicketReport {
                ticket_id: "tt-one".to_string(),
                path: PathBuf::from(".waap/tickets/tt-one/ticket.md"),
                title: "One".to_string(),
                creation_date: "2026-06-22T12:00:00Z".to_string(),
                status: "completed".to_string(),
                depends_on: None,
                file_size: 123,
            },
            blocked: false,
        }];

        let lines = ticket_list_human_lines(&entries);

        assert_eq!(lines[0], "TICKET ID  STATUS");
        assert_eq!(lines[1], "tt-one     completed");
    }

    #[test]
    fn ticket_list_human_lines_header_widens_for_short_ticket_ids() {
        let entries = vec![TicketListEntry {
            report: TicketReport {
                ticket_id: "tt".to_string(),
                path: PathBuf::from(".waap/tickets/tt/ticket.md"),
                title: "Short".to_string(),
                creation_date: "2026-06-22T12:00:00Z".to_string(),
                status: "pending".to_string(),
                depends_on: None,
                file_size: 1,
            },
            blocked: false,
        }];

        let lines = ticket_list_human_lines(&entries);

        assert_eq!(lines[0], "TICKET ID  STATUS");
        assert_eq!(lines[1], "tt         pending");
    }

    #[test]
    fn ticket_list_human_lines_empty_has_no_header() {
        let entries: Vec<TicketListEntry> = Vec::new();

        let lines = ticket_list_human_lines(&entries);

        assert!(lines.is_empty());
    }

    #[test]
    fn ticket_list_all_unblocked_when_no_deps() {
        let dir = tempdir().unwrap();
        write_ticket(dir.path(), "tt-alpha", "pending", &[]);
        write_ticket(dir.path(), "tt-beta", "in-progress", &[]);

        let entries = list_tickets(dir.path(), None, None).unwrap();

        assert!(entries.iter().all(|e| !e.blocked));
    }

    #[test]
    fn ticket_list_blocked_when_dep_not_completed() {
        let dir = tempdir().unwrap();
        write_ticket(dir.path(), "tt-dep", "in-progress", &[]);
        write_ticket(dir.path(), "tt-feature", "pending", &["tt-dep"]);

        let entries = list_tickets(dir.path(), None, None).unwrap();

        let feature = entries
            .iter()
            .find(|e| e.report.ticket_id == "tt-feature")
            .unwrap();
        assert!(feature.blocked);
        let dep = entries
            .iter()
            .find(|e| e.report.ticket_id == "tt-dep")
            .unwrap();
        assert!(!dep.blocked);
    }

    #[test]
    fn ticket_list_unblocked_when_dep_completed() {
        let dir = tempdir().unwrap();
        write_ticket(dir.path(), "tt-dep", "completed", &[]);
        write_ticket(dir.path(), "tt-feature", "pending", &["tt-dep"]);

        let entries = list_tickets(dir.path(), None, None).unwrap();

        let feature = entries
            .iter()
            .find(|e| e.report.ticket_id == "tt-feature")
            .unwrap();
        assert!(!feature.blocked);
    }

    #[test]
    fn ticket_list_filter_blocked_returns_only_blocked() {
        let dir = tempdir().unwrap();
        write_ticket(dir.path(), "tt-dep", "pending", &[]);
        write_ticket(dir.path(), "tt-blocked", "pending", &["tt-dep"]);
        write_ticket(dir.path(), "tt-free", "pending", &[]);

        let entries = list_tickets(dir.path(), None, Some(true)).unwrap();

        assert_eq!(ticket_ids(&entries), vec!["tt-blocked"]);
    }

    #[test]
    fn ticket_list_filter_unblocked_returns_only_unblocked() {
        let dir = tempdir().unwrap();
        write_ticket(dir.path(), "tt-dep", "pending", &[]);
        write_ticket(dir.path(), "tt-blocked", "pending", &["tt-dep"]);
        write_ticket(dir.path(), "tt-free", "pending", &[]);

        let entries = list_tickets(dir.path(), None, Some(false)).unwrap();

        let ids = ticket_ids(&entries);
        assert!(ids.contains(&"tt-dep"));
        assert!(ids.contains(&"tt-free"));
        assert!(!ids.contains(&"tt-blocked"));
    }

    fn ticket_ids(entries: &[TicketListEntry]) -> Vec<&str> {
        entries
            .iter()
            .map(|entry| entry.report.ticket_id.as_str())
            .collect()
    }

    fn write_ticket(waap_root: &Path, ticket_id: &str, status: &str, depends_on: &[&str]) {
        write_ticket_full(
            waap_root,
            ticket_id,
            status,
            depends_on,
            "2026-06-18T15:00:34Z",
        );
    }

    fn write_ticket_with_creation_date(
        waap_root: &Path,
        ticket_id: &str,
        status: &str,
        creation_date: &str,
    ) {
        write_ticket_full(waap_root, ticket_id, status, &[], creation_date);
    }

    fn write_ticket_full(
        waap_root: &Path,
        ticket_id: &str,
        status: &str,
        depends_on: &[&str],
        creation_date: &str,
    ) {
        let deps_line = if depends_on.is_empty() {
            String::new()
        } else {
            let items: Vec<String> = depends_on.iter().map(|d| format!("\"{d}\"")).collect();
            format!("depends_on = [{}]\n", items.join(", "))
        };
        write_file(
            &waap_root.join(format!(".waap/tickets/{ticket_id}/ticket.md")),
            &format!(
                "+++\ntitle = \"Test Ticket\"\ncreation_date = {creation_date}\nstatus = \"{status}\"\n{deps_line}+++\n\n# Description\n"
            ),
        );
    }

    fn write_file(path: &Path, contents: &str) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, contents).unwrap();
    }
}
