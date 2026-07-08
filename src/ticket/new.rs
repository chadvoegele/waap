use std::collections::HashSet;
use std::fs;
use std::io::{self, Read};
use std::path::Path;

use crate::cli::OutputFormat;
use crate::git::{commit_paths, Committed};
use crate::record::WaapRecordKind;
use crate::ticket::{
    available_ticket_id, is_ticket_id, load_tickets_metadata, print_ticket_report_human,
    ticket_path, ticket_report_json, write_ticket_record, TicketMetadata, TicketReport,
};
use crate::toml::current_toml_datetime;

pub(crate) fn print_ticket_report(
    output_format: &OutputFormat,
    committed: &Committed<TicketReport>,
) {
    let report = &committed.value;
    match output_format {
        OutputFormat::Json => {
            let mut value = ticket_report_json(report);
            value["commit"] = serde_json::json!(committed.commit);
            println!("{value}");
        }
        OutputFormat::HumanReadable => {
            print_ticket_report_human("Created ticket", report);
            println!("Commit: {}", committed.commit);
        }
    }
}

pub(crate) fn create_ticket(
    waap_root: &Path,
    name: Option<&str>,
    depends_on: &[String],
) -> io::Result<Committed<TicketReport>> {
    let mut markdown = String::new();
    io::stdin()
        .read_to_string(&mut markdown)
        .map_err(|error| io::Error::new(error.kind(), format!("failed to read stdin: {error}")))?;

    let report = create_ticket_with_markdown(waap_root, name, depends_on, &markdown)?;
    let commit = commit_paths(
        waap_root,
        &[report.path.as_path()],
        &format!("waap ticket new {}", report.ticket_id),
    )
    .map_err(|error| {
        io::Error::new(
            error.kind(),
            format!("failed to commit waap state change: {error}"),
        )
    })?;

    Ok(Committed {
        value: report,
        commit,
    })
}

pub(crate) fn create_ticket_with_markdown(
    waap_root: &Path,
    name: Option<&str>,
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

    let ticket_ids: HashSet<String> = load_tickets_metadata(waap_root)?
        .into_iter()
        .map(|metadata| metadata.ticket_id)
        .collect();
    for id in depends_on {
        if !ticket_ids.contains(id) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("dependency ticket {id:?} does not exist"),
            ));
        }
    }

    let tickets_dir = WaapRecordKind::Ticket.root_path(waap_root);
    let ticket_id = available_ticket_id(&tickets_dir, name)?;

    let depends_on_opt = if depends_on.is_empty() {
        None
    } else {
        Some(depends_on.to_vec())
    };

    let creation_date = current_toml_datetime();
    let metadata = TicketMetadata {
        ticket_id: ticket_id.clone(),
        name: name.map(str::to_string),
        creation_date: creation_date.clone(),
        status: "pending".to_string(),
        depends_on: depends_on_opt,
    };
    write_ticket_record(waap_root, &ticket_id, &metadata, &format!("\n{markdown}"))?;
    let path = ticket_path(waap_root, &ticket_id);
    let file_size = fs::metadata(&path)?.len();

    Ok(TicketReport {
        ticket_id,
        path,
        name: metadata.name,
        creation_date,
        status: "pending".to_string(),
        depends_on: metadata.depends_on,
        file_size,
    })
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::Path;

    use tempfile::tempdir;

    use super::create_ticket_with_markdown;
    use crate::check::check_waap;

    fn create_named_ticket(waap_root: &Path, name: &str) {
        create_ticket_with_markdown(waap_root, Some(name), &[], "").unwrap();
    }

    #[test]
    fn create_ticket_writes_frontmatter_and_stdin_content() {
        let dir = tempdir().unwrap();
        fs::create_dir_all(dir.path().join(".waap")).unwrap();

        let report =
            create_ticket_with_markdown(dir.path(), Some("New Ticket"), &[], "# Body\nDetails\n")
                .unwrap();
        let contents = fs::read_to_string(&report.path).unwrap();

        assert_eq!(report.ticket_id, "tt-new-ticket");
        assert_eq!(report.name.as_deref(), Some("New Ticket"));
        assert_eq!(report.status, "pending");
        assert_eq!(report.file_size, contents.len() as u64);
        assert!(contents.starts_with("+++\nname = \"New Ticket\"\ncreation_date = "));
        assert!(contents.contains("\nstatus = \"pending\"\n+++\n\n# Body\nDetails\n"));
        assert!(check_waap(dir.path()).is_empty());
    }

    #[test]
    fn create_ticket_with_depends_on_round_trips() {
        let dir = tempdir().unwrap();
        fs::create_dir_all(dir.path().join(".waap")).unwrap();
        create_named_ticket(dir.path(), "Dep One");
        create_named_ticket(dir.path(), "Dep Two");

        let deps = vec!["tt-dep-one".to_string(), "tt-dep-two".to_string()];
        let report =
            create_ticket_with_markdown(dir.path(), Some("Dependent Ticket"), &deps, "# Body\n")
                .unwrap();
        let contents = fs::read_to_string(&report.path).unwrap();

        assert_eq!(report.depends_on, Some(deps));
        assert!(contents.contains("depends_on = [\"tt-dep-one\", \"tt-dep-two\"]"));
    }

    #[test]
    fn create_ticket_rejects_invalid_depends_on_id() {
        let dir = tempdir().unwrap();
        fs::create_dir_all(dir.path().join(".waap")).unwrap();

        let deps = vec!["not-a-ticket-id".to_string()];
        let err = create_ticket_with_markdown(dir.path(), Some("Bad Deps"), &deps, "").unwrap_err();

        assert_eq!(err.kind(), std::io::ErrorKind::InvalidInput);
        assert!(err.to_string().contains("not-a-ticket-id"));
    }

    #[test]
    fn create_ticket_rejects_missing_depends_on_ticket() {
        let dir = tempdir().unwrap();
        fs::create_dir_all(dir.path().join(".waap")).unwrap();

        let deps = vec!["tt-missing".to_string()];
        let err = create_ticket_with_markdown(dir.path(), Some("Bad Deps"), &deps, "").unwrap_err();

        assert_eq!(err.kind(), std::io::ErrorKind::InvalidInput);
        assert!(err.to_string().contains("tt-missing"));
        assert!(!dir.path().join(".waap/tickets/tt-bad-deps").exists());
    }

    #[test]
    fn create_ticket_without_name_uses_random_hex_id() {
        let dir = tempdir().unwrap();
        fs::create_dir_all(dir.path().join(".waap")).unwrap();

        let report = create_ticket_with_markdown(dir.path(), None, &[], "# Body\n").unwrap();
        let suffix = report.ticket_id.strip_prefix("tt-").unwrap();
        let contents = fs::read_to_string(&report.path).unwrap();

        assert_eq!(suffix.len(), 8);
        assert!(suffix.bytes().all(|byte| byte.is_ascii_hexdigit()));
        assert_eq!(report.name, None);
        assert!(!contents.contains("name ="));
    }
}
