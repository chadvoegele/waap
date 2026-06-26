use std::fs;
use std::path::Path;

use serde_json::json;

use crate::agent::check_agent_frontmatter;
use crate::agent::is_agent_id;
use crate::cli::OutputFormat;
use crate::ticket::check_ticket_frontmatter;
use crate::ticket::is_ticket_id;

pub(crate) fn check_waap(repo_root: &Path) -> Vec<String> {
    let mut errors = Vec::new();
    let waap_dir = repo_root.join(".waap");
    let agents_dir = waap_dir.join("agents");
    let tickets_dir = waap_dir.join("tickets");

    if !waap_dir.exists() {
        return errors;
    }

    if !waap_dir.is_dir() {
        errors.push(".waap must be a directory".to_string());
        return errors;
    }

    if agents_dir.exists() && agents_dir.is_dir() {
        check_agents(&agents_dir, &mut errors);
    } else if agents_dir.exists() {
        errors.push(".waap/agents must be a directory".to_string());
    }

    if tickets_dir.exists() && tickets_dir.is_dir() {
        check_tickets(&tickets_dir, &mut errors);
    } else if tickets_dir.exists() {
        errors.push(".waap/tickets must be a directory".to_string());
    }

    errors
}

pub(crate) fn print_check_result(output_format: &OutputFormat, errors: &[String]) {
    match output_format {
        OutputFormat::Json => {
            println!(
                "{}",
                json!({
                    "valid": errors.is_empty(),
                    "errors": errors,
                })
            );
        }
        OutputFormat::HumanReadable => {
            if errors.is_empty() {
                println!("OK: .waap is valid");
            } else {
                println!("ERROR: .waap is invalid");
                for error in errors {
                    println!("- {error}");
                }
            }
        }
    }
}

pub(crate) fn check_agents(agents_dir: &Path, errors: &mut Vec<String>) {
    let entries = read_dir(agents_dir, ".waap/agents", errors);
    for entry in entries {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().into_owned();
        let label = format!(".waap/agents/{name}");

        if !path.is_dir() {
            errors.push(format!("{label} must be an agent directory"));
            continue;
        }

        if !is_agent_id(&name) {
            errors.push(format!(
                "{label} must be named as an agent id like aa-3881fda0"
            ));
        }

        let agent_file = path.join("agent.md");
        if !agent_file.is_file() {
            errors.push(format!("{label}/agent.md is required"));
        } else {
            check_agent_frontmatter(&agent_file, errors);
        }
    }
}

pub(crate) fn check_tickets(tickets_dir: &Path, errors: &mut Vec<String>) {
    let entries = read_dir(tickets_dir, ".waap/tickets", errors);
    for entry in entries {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().into_owned();
        let label = format!(".waap/tickets/{name}");

        if !path.is_dir() {
            errors.push(format!("{label} must be a ticket directory"));
            continue;
        }

        if !is_ticket_id(&name) {
            errors.push(format!(
                "{label} must be named as a ticket id like tt-list-tickets"
            ));
        }

        let ticket_file = path.join("ticket.md");
        if !ticket_file.is_file() {
            errors.push(format!("{label}/ticket.md is required"));
        } else {
            check_ticket_frontmatter(&ticket_file, errors);
        }
    }
}

pub(crate) fn read_dir(path: &Path, label: &str, errors: &mut Vec<String>) -> Vec<fs::DirEntry> {
    match fs::read_dir(path) {
        Ok(entries) => entries
            .filter_map(|entry| match entry {
                Ok(entry) => Some(entry),
                Err(error) => {
                    errors.push(format!("failed to read entry in {label}: {error}"));
                    None
                }
            })
            .collect(),
        Err(error) => {
            errors.push(format!("failed to read {label}: {error}"));
            Vec::new()
        }
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::Path;

    use tempfile::tempdir;

    use super::check_waap;

    #[test]
    fn valid_waap_state_passes() {
        let dir = tempdir().unwrap();
        write_file(
            &dir.path().join(".waap/agents/aa-3881fda0/agent.md"),
            "+++\ncreation_date = 2026-06-18T15:00:34Z\nrole = \"developer\"\nstatus = \"ready\"\n+++\n\n# Purpose\n",
        );
        write_file(
            &dir.path().join(".waap/agents/aa-3881fda0/work_log.md"),
            "# Work Log\n",
        );
        write_file(
            &dir.path().join(".waap/tickets/tt-list-tickets/ticket.md"),
            "+++\ntitle = \"List Tickets\"\ncreation_date = 2026-06-18T10:15:02Z\nstatus = \"pending\"\n+++\n\n# Description\n",
        );

        assert!(check_waap(dir.path()).is_empty());
    }

    #[test]
    fn missing_waap_state_passes() {
        let dir = tempdir().unwrap();

        assert!(check_waap(dir.path()).is_empty());
    }

    #[test]
    fn missing_child_directories_pass() {
        let dir = tempdir().unwrap();
        fs::create_dir_all(dir.path().join(".waap")).unwrap();

        assert!(check_waap(dir.path()).is_empty());
    }

    #[test]
    fn agent_directories_allow_extra_files() {
        let dir = tempdir().unwrap();
        write_file(
            &dir.path().join(".waap/agents/aa-3881fda0/agent.md"),
            "+++\ncreation_date = 2026-06-18T15:00:34Z\nrole = \"developer\"\nstatus = \"ready\"\n+++\n",
        );
        write_file(
            &dir.path().join(".waap/agents/aa-3881fda0/notes.md"),
            "# Notes\n",
        );

        assert!(check_waap(dir.path()).is_empty());
    }

    #[test]
    fn existing_state_paths_must_be_directories() {
        let dir = tempdir().unwrap();
        write_file(&dir.path().join(".waap"), "not a directory");

        let errors = check_waap(dir.path());

        assert_eq!(errors, vec![".waap must be a directory"]);

        let dir = tempdir().unwrap();
        fs::create_dir_all(dir.path().join(".waap")).unwrap();
        write_file(&dir.path().join(".waap/agents"), "not a directory");
        write_file(&dir.path().join(".waap/tickets"), "not a directory");

        let errors = check_waap(dir.path());

        assert!(errors.contains(&".waap/agents must be a directory".to_string()));
        assert!(errors.contains(&".waap/tickets must be a directory".to_string()));
    }

    #[test]
    fn invalid_agent_frontmatter_fails() {
        let dir = tempdir().unwrap();
        fs::create_dir_all(dir.path().join(".waap/tickets")).unwrap();
        write_file(
            &dir.path().join(".waap/agents/aa-3881fda0/agent.md"),
            "+++\ncreation_date = \"not a datetime\"\nrole = \"designer\"\nstatus = \"ready\"\n+++\n",
        );

        let errors = check_waap(dir.path());

        assert!(errors
            .iter()
            .any(|error| error.contains("creation_date must be a TOML datetime")));
        assert!(errors
            .iter()
            .any(|error| error.contains("role has invalid value")));
    }

    #[test]
    fn invalid_ticket_id_fails() {
        let dir = tempdir().unwrap();
        fs::create_dir_all(dir.path().join(".waap/agents")).unwrap();
        write_file(
            &dir.path().join(".waap/tickets/tt-Bad--Ticket/ticket.md"),
            "+++\ntitle = \"Bad Ticket\"\ncreation_date = 2026-06-18T10:15:02Z\nstatus = \"pending\"\n+++\n",
        );

        let errors = check_waap(dir.path());

        assert!(errors
            .iter()
            .any(|error| error.contains("must be named as a ticket id")));
    }

    fn write_file(path: &Path, contents: &str) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, contents).unwrap();
    }
}
