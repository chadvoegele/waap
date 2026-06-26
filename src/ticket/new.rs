use std::fs;
use std::io::{self, Read};
use std::path::Path;

use crate::cli::OutputFormat;
use crate::ids::current_toml_datetime;
use crate::record::WaapRecordKind;
use crate::ticket::{
    available_ticket_id, print_ticket_report_human, ticket_path, ticket_report_json,
    write_ticket_record, TicketMetadata, TicketReport,
};

pub(crate) fn print_ticket_report(output_format: &OutputFormat, report: &TicketReport) {
    match output_format {
        OutputFormat::Json => println!("{}", ticket_report_json(report)),
        OutputFormat::HumanReadable => print_ticket_report_human("Created ticket", report),
    }
}

pub(crate) fn create_ticket(repo_root: &Path, title: &str) -> io::Result<TicketReport> {
    let mut markdown = String::new();
    io::stdin()
        .read_to_string(&mut markdown)
        .map_err(|error| io::Error::new(error.kind(), format!("failed to read stdin: {error}")))?;

    create_ticket_with_markdown(repo_root, title, &markdown)
}

pub(crate) fn create_ticket_with_markdown(
    repo_root: &Path,
    title: &str,
    markdown: &str,
) -> io::Result<TicketReport> {
    let tickets_dir = WaapRecordKind::Ticket.root_path(repo_root);
    let ticket_id = available_ticket_id(&tickets_dir, title)?;

    let creation_date = current_toml_datetime();
    let metadata = TicketMetadata {
        title: title.to_string(),
        creation_date: creation_date.clone(),
        status: "pending".to_string(),
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
            create_ticket_with_markdown(dir.path(), "New Ticket", "# Body\nDetails\n").unwrap();
        let contents = fs::read_to_string(&report.path).unwrap();

        assert_eq!(report.ticket_id, "tt-new-ticket");
        assert_eq!(report.title, "New Ticket");
        assert_eq!(report.status, "pending");
        assert_eq!(report.file_size, contents.len() as u64);
        assert!(contents.starts_with("+++\ntitle = \"New Ticket\"\ncreation_date = "));
        assert!(contents.contains("\nstatus = \"pending\"\n+++\n\n# Body\nDetails\n"));
        assert!(check_waap(dir.path()).is_empty());
    }
}
