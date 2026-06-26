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
            for entry in entries {
                if entry.report.depends_on.is_some() {
                    let state = if entry.blocked {
                        "[blocked]"
                    } else {
                        "[unblocked]"
                    };
                    println!("{} {state}", entry.report.ticket_id);
                } else {
                    println!("{}", entry.report.ticket_id);
                }
            }
        }
    }
}

pub(crate) fn ticket_list_json(entries: &[TicketListEntry]) -> serde_json::Value {
    json!(entries
        .iter()
        .map(|entry| json!({"ticket_id": entry.report.ticket_id, "blocked": entry.blocked}))
        .collect::<Vec<_>>())
}

pub(crate) fn list_tickets(
    repo_root: &Path,
    status: Option<&TicketStatus>,
    blocked_filter: Option<bool>,
) -> io::Result<Vec<TicketListEntry>> {
    let all_ids = list_record_ids(repo_root, WaapRecordKind::Ticket)?;

    let mut all_reports: Vec<TicketReport> = Vec::new();
    for ticket_id in &all_ids {
        all_reports.push(load_ticket_report(repo_root, ticket_id)?);
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

    Ok(entries)
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::io;
    use std::path::{Path, PathBuf};

    use serde_json::json;
    use tempfile::tempdir;

    use super::{list_tickets, ticket_list_json, TicketListEntry};
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
                {"ticket_id": "tt-one", "blocked": false},
                {"ticket_id": "tt-two", "blocked": false},
            ])
        );
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

    fn ticket_ids<'a>(entries: &'a [TicketListEntry]) -> Vec<&'a str> {
        entries
            .iter()
            .map(|entry| entry.report.ticket_id.as_str())
            .collect()
    }

    fn write_ticket(repo_root: &Path, ticket_id: &str, status: &str, depends_on: &[&str]) {
        let deps_line = if depends_on.is_empty() {
            String::new()
        } else {
            let items: Vec<String> = depends_on.iter().map(|d| format!("\"{d}\"")).collect();
            format!("depends_on = [{}]\n", items.join(", "))
        };
        write_file(
            &repo_root.join(format!(".waap/tickets/{ticket_id}/ticket.md")),
            &format!(
                "+++\ntitle = \"Test Ticket\"\ncreation_date = 2026-06-18T15:00:34Z\nstatus = \"{status}\"\n{deps_line}+++\n\n# Description\n"
            ),
        );
    }

    fn write_file(path: &Path, contents: &str) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, contents).unwrap();
    }
}
