use std::fs;
use std::io::{self, Read};
use std::path::Path;

use crate::cli::OutputFormat;
use crate::ids::current_toml_datetime;
use crate::record::WaapRecordKind;
use crate::ticket::{
    available_ticket_id, is_ticket_id, print_ticket_report_human, ticket_path, ticket_report_json,
    write_ticket_record, TicketMetadata, TicketReport,
};

pub(crate) fn print_ticket_report(
    output_format: &OutputFormat,
    report: &TicketReport,
    commit: &str,
) {
    match output_format {
        OutputFormat::Json => {
            let mut value = ticket_report_json(report);
            value["commit"] = serde_json::json!(commit);
            println!("{value}");
        }
        OutputFormat::HumanReadable => {
            print_ticket_report_human("Created ticket", report);
            println!("Commit: {commit}");
        }
    }
}

pub(crate) fn create_ticket(
    repo_root: &Path,
    title: &str,
    depends_on: &[String],
) -> io::Result<TicketReport> {
    let mut markdown = String::new();
    io::stdin()
        .read_to_string(&mut markdown)
        .map_err(|error| io::Error::new(error.kind(), format!("failed to read stdin: {error}")))?;

    create_ticket_with_markdown(repo_root, title, depends_on, &markdown)
}

pub(crate) fn create_ticket_with_markdown(
    repo_root: &Path,
    title: &str,
    depends_on: &[String],
    markdown: &str,
) -> io::Result<TicketReport> {
    for id in depends_on {
        if !is_ticket_id(id) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("{id:?} is not a valid ticket id"),
            ));
        }
    }

    let tickets_dir = WaapRecordKind::Ticket.root_path(repo_root);
    let ticket_id = available_ticket_id(&tickets_dir, title)?;

    let depends_on_opt = if depends_on.is_empty() {
        None
    } else {
        Some(depends_on.to_vec())
    };

    let creation_date = current_toml_datetime();
    let metadata = TicketMetadata {
        title: title.to_string(),
        creation_date: creation_date.clone(),
        status: "pending".to_string(),
        depends_on: depends_on_opt,
    };
    write_ticket_record(repo_root, &ticket_id, &metadata, &format!("\n{markdown}"))?;
    let path = ticket_path(repo_root, &ticket_id);
    let file_size = fs::metadata(&path)?.len();

    Ok(TicketReport {
        ticket_id,
        path,
        title: title.to_string(),
        creation_date,
        status: "pending".to_string(),
        depends_on: metadata.depends_on,
        file_size,
    })
}

#[cfg(test)]
mod tests {
    use std::fs;

    use tempfile::tempdir;

    use super::create_ticket_with_markdown;
    use crate::check::check_waap;

    #[test]
    fn create_ticket_writes_frontmatter_and_stdin_content() {
        let dir = tempdir().unwrap();

        let report =
            create_ticket_with_markdown(dir.path(), "New Ticket", &[], "# Body\nDetails\n")
                .unwrap();
        let contents = fs::read_to_string(&report.path).unwrap();

        assert_eq!(report.ticket_id, "tt-new-ticket");
        assert_eq!(report.title, "New Ticket");
        assert_eq!(report.status, "pending");
        assert_eq!(report.file_size, contents.len() as u64);
        assert!(contents.starts_with("+++\ntitle = \"New Ticket\"\ncreation_date = "));
        assert!(contents.contains("\nstatus = \"pending\"\n+++\n\n# Body\nDetails\n"));
        assert!(check_waap(dir.path()).is_empty());
    }

    #[test]
    fn create_ticket_with_depends_on_round_trips() {
        let dir = tempdir().unwrap();

        let deps = vec!["tt-dep-one".to_string(), "tt-dep-two".to_string()];
        let report =
            create_ticket_with_markdown(dir.path(), "Dependent Ticket", &deps, "# Body\n").unwrap();
        let contents = fs::read_to_string(&report.path).unwrap();

        assert_eq!(report.depends_on, Some(deps));
        assert!(contents.contains("depends_on = [\"tt-dep-one\", \"tt-dep-two\"]"));
    }

    #[test]
    fn create_ticket_rejects_invalid_depends_on_id() {
        let dir = tempdir().unwrap();

        let deps = vec!["not-a-ticket-id".to_string()];
        let err = create_ticket_with_markdown(dir.path(), "Bad Deps", &deps, "").unwrap_err();

        assert_eq!(err.kind(), std::io::ErrorKind::InvalidInput);
        assert!(err.to_string().contains("not-a-ticket-id"));
    }
}
