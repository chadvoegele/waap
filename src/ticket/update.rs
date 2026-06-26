use std::fs;
use std::io;
use std::path::Path;

use crate::cli::OutputFormat;
use crate::ticket::{
    is_ticket_id, print_ticket_report_human, read_ticket_record, ticket_path, ticket_report_json,
    write_ticket_record, TicketReport, TicketStatus,
};

pub(crate) fn print_updated_ticket_report(output_format: &OutputFormat, report: &TicketReport) {
    match output_format {
        OutputFormat::Json => println!("{}", ticket_report_json(report)),
        OutputFormat::HumanReadable => print_ticket_report_human("Updated ticket", report),
    }
}

pub(crate) fn update_ticket(
    repo_root: &Path,
    ticket_id: &str,
    set_status: Option<&TicketStatus>,
    add_depends_on: &[String],
    remove_depends_on: &[String],
) -> io::Result<TicketReport> {
    for dep_id in add_depends_on.iter().chain(remove_depends_on.iter()) {
        if !is_ticket_id(dep_id) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("{dep_id:?} is not a valid ticket id"),
            ));
        }
    }

    let (mut metadata, body) = read_ticket_record(repo_root, ticket_id)?;

    if let Some(status) = set_status {
        metadata.status = status.as_str().to_string();
    }

    let mut deps: Vec<String> = metadata.depends_on.unwrap_or_default();
    for dep_id in add_depends_on {
        if !deps.contains(dep_id) {
            deps.push(dep_id.clone());
        }
    }
    for dep_id in remove_depends_on {
        deps.retain(|d| d != dep_id);
    }
    metadata.depends_on = if deps.is_empty() { None } else { Some(deps) };

    write_ticket_record(repo_root, ticket_id, &metadata, &body)?;

    let path = ticket_path(repo_root, ticket_id);
    Ok(TicketReport {
        ticket_id: ticket_id.to_string(),
        path: path.clone(),
        title: metadata.title,
        creation_date: metadata.creation_date,
        status: metadata.status,
        depends_on: metadata.depends_on,
        file_size: fs::metadata(&path)?.len(),
    })
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::io;
    use std::path::{Path, PathBuf};

    use serde_json::json;
    use tempfile::tempdir;

    use crate::ticket::{ticket_report_json, update_ticket, TicketReport, TicketStatus};

    fn write_file(path: &Path, contents: &str) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, contents).unwrap();
    }

    fn make_ticket(dir: &Path, ticket_id: &str, extra_frontmatter: &str) {
        let path = dir.join(format!(".waap/tickets/{ticket_id}/ticket.md"));
        write_file(
            &path,
            &format!(
                "+++\ntitle = \"Test\"\ncreation_date = 2026-06-22T12:00:00Z\nstatus = \"pending\"\n{extra_frontmatter}+++\n\n# Body\n"
            ),
        );
    }

    #[test]
    fn ticket_update_reports_missing_ticket() {
        let dir = tempdir().unwrap();

        let error = update_ticket(
            dir.path(),
            "tt-new-ticket",
            Some(&TicketStatus::Completed),
            &[],
            &[],
        )
        .unwrap_err();

        assert_eq!(error.kind(), io::ErrorKind::NotFound);
        assert!(error
            .to_string()
            .contains(".waap/tickets/tt-new-ticket/ticket.md"));
    }

    #[test]
    fn ticket_update_rejects_invalid_ticket_id() {
        let dir = tempdir().unwrap();

        let error = update_ticket(
            dir.path(),
            "new-ticket",
            Some(&TicketStatus::Completed),
            &[],
            &[],
        )
        .unwrap_err();

        assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
        assert!(error.to_string().contains("not a valid ticket id"));
    }

    #[test]
    fn ticket_update_preserves_frontmatter_and_body_except_status() {
        let dir = tempdir().unwrap();
        let path = dir.path().join(".waap/tickets/tt-new-ticket/ticket.md");
        let body = "# Description\nKeep this body exactly.\n";
        write_file(
            &path,
            &format!(
                "+++\ntitle = \"New Ticket\"\ncreation_date = 2026-06-22T12:00:00Z\nstatus = \"pending\"\n+++\n\n{body}"
            ),
        );

        let report = update_ticket(
            dir.path(),
            "tt-new-ticket",
            Some(&TicketStatus::Completed),
            &[],
            &[],
        )
        .unwrap();
        let contents = fs::read_to_string(&path).unwrap();

        assert_eq!(report.ticket_id, "tt-new-ticket");
        assert_eq!(report.title, "New Ticket");
        assert_eq!(report.creation_date, "2026-06-22T12:00:00Z");
        assert_eq!(report.status, "completed");
        assert_eq!(report.file_size, contents.len() as u64);
        assert_eq!(
            contents,
            format!(
                "+++\ntitle = \"New Ticket\"\ncreation_date = 2026-06-22T12:00:00Z\nstatus = \"completed\"\n+++\n\n{body}"
            )
        );
    }

    #[test]
    fn ticket_report_json_has_expected_shape() {
        let report = TicketReport {
            ticket_id: "tt-new-ticket".to_string(),
            path: PathBuf::from(".waap/tickets/tt-new-ticket/ticket.md"),
            title: "New Ticket".to_string(),
            creation_date: "2026-06-22T12:00:00Z".to_string(),
            status: "pending".to_string(),
            depends_on: None,
            file_size: 123,
        };

        assert_eq!(
            ticket_report_json(&report),
            json!({
                "ticket_id": "tt-new-ticket",
                "path": ".waap/tickets/tt-new-ticket/ticket.md",
                "metadata": {
                    "title": "New Ticket",
                    "creation_date": "2026-06-22T12:00:00Z",
                    "status": "pending",
                    "depends_on": null,
                },
                "file_size": 123,
            })
        );
    }

    #[test]
    fn add_depends_on_adds_new_dependency() {
        let dir = tempdir().unwrap();
        make_ticket(dir.path(), "tt-my-ticket", "");

        let report = update_ticket(
            dir.path(),
            "tt-my-ticket",
            None,
            &["tt-dep-a".to_string()],
            &[],
        )
        .unwrap();

        assert_eq!(report.depends_on, Some(vec!["tt-dep-a".to_string()]));
        let contents =
            fs::read_to_string(dir.path().join(".waap/tickets/tt-my-ticket/ticket.md")).unwrap();
        assert!(contents.contains("depends_on = [\"tt-dep-a\"]"));
    }

    #[test]
    fn remove_depends_on_removes_existing_dependency() {
        let dir = tempdir().unwrap();
        make_ticket(
            dir.path(),
            "tt-my-ticket",
            "depends_on = [\"tt-dep-a\", \"tt-dep-b\"]\n",
        );

        let report = update_ticket(
            dir.path(),
            "tt-my-ticket",
            None,
            &[],
            &["tt-dep-a".to_string()],
        )
        .unwrap();

        assert_eq!(report.depends_on, Some(vec!["tt-dep-b".to_string()]));
    }

    #[test]
    fn add_and_remove_depends_on_in_one_call() {
        let dir = tempdir().unwrap();
        make_ticket(dir.path(), "tt-my-ticket", "depends_on = [\"tt-dep-a\"]\n");

        let report = update_ticket(
            dir.path(),
            "tt-my-ticket",
            None,
            &["tt-dep-b".to_string()],
            &["tt-dep-a".to_string()],
        )
        .unwrap();

        assert_eq!(report.depends_on, Some(vec!["tt-dep-b".to_string()]));
    }

    #[test]
    fn remove_nonexistent_depends_on_is_noop() {
        let dir = tempdir().unwrap();
        make_ticket(dir.path(), "tt-my-ticket", "depends_on = [\"tt-dep-a\"]\n");

        let report = update_ticket(
            dir.path(),
            "tt-my-ticket",
            None,
            &[],
            &["tt-dep-x".to_string()],
        )
        .unwrap();

        assert_eq!(report.depends_on, Some(vec!["tt-dep-a".to_string()]));
    }

    #[test]
    fn add_depends_on_rejects_invalid_ticket_id() {
        let dir = tempdir().unwrap();
        make_ticket(dir.path(), "tt-my-ticket", "");

        let error = update_ticket(
            dir.path(),
            "tt-my-ticket",
            None,
            &["not-a-ticket-id".to_string()],
            &[],
        )
        .unwrap_err();

        assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
        assert!(error.to_string().contains("not a valid ticket id"));
    }

    #[test]
    fn remove_depends_on_rejects_invalid_ticket_id() {
        let dir = tempdir().unwrap();
        make_ticket(dir.path(), "tt-my-ticket", "");

        let error = update_ticket(
            dir.path(),
            "tt-my-ticket",
            None,
            &[],
            &["not-a-ticket-id".to_string()],
        )
        .unwrap_err();

        assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
        assert!(error.to_string().contains("not a valid ticket id"));
    }
}
