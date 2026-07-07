use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::Path;

use serde_json::json;

use crate::agent::check_agent_frontmatter;
use crate::agent::is_agent_id;
use crate::cli::OutputFormat;
use crate::frontmatter::parse_frontmatter;
use crate::ticket::{is_ticket_id, TicketMetadata};

pub(crate) fn check_waap(waap_root: &Path) -> Vec<String> {
    let mut errors = Vec::new();
    let waap_dir = waap_root.join(".waap");
    let agents_dir = waap_dir.join("agents");
    let tickets_dir = waap_dir.join("tickets");

    if !waap_dir.exists() {
        errors.push("no waap project found; run 'waap init'".to_string());
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
    println!("{}", format_check_result(output_format, errors));
}

pub(crate) fn print_check_errors(output_format: &OutputFormat, errors: &[String]) {
    eprintln!("{}", format_check_result(output_format, errors));
}

fn format_check_result(output_format: &OutputFormat, errors: &[String]) -> String {
    match output_format {
        OutputFormat::Json => json!({
            "valid": errors.is_empty(),
            "errors": errors,
        })
        .to_string(),
        OutputFormat::HumanReadable => {
            if errors.is_empty() {
                "OK: .waap is valid".to_string()
            } else {
                let mut output = "ERROR: .waap is invalid".to_string();
                for error in errors {
                    output.push_str(&format!("\n- {error}"));
                }
                output
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
    let mut known_ids: HashSet<String> = HashSet::new();
    let mut deps_map: HashMap<String, Vec<String>> = HashMap::new();

    for entry in entries {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().into_owned();
        let label = format!(".waap/tickets/{name}");

        if !path.is_dir() {
            errors.push(format!("{label} must be a ticket directory"));
            continue;
        }

        if is_ticket_id(&name) {
            known_ids.insert(name.clone());
        } else {
            errors.push(format!(
                "{label} must be named as a ticket id like tt-list-tickets"
            ));
        }

        let ticket_file = path.join("ticket.md");
        if !ticket_file.is_file() {
            errors.push(format!("{label}/ticket.md is required"));
        } else if let Some(frontmatter) = parse_frontmatter(&ticket_file, errors) {
            match TicketMetadata::from_frontmatter(&frontmatter, &ticket_file) {
                Ok(metadata) => {
                    if let Some(deps) = metadata.depends_on {
                        deps_map.insert(name.clone(), deps);
                    }
                }
                Err(mut frontmatter_errors) => errors.append(&mut frontmatter_errors),
            }
        }
    }

    check_ticket_dependencies(&known_ids, &deps_map, errors);
}

fn check_ticket_dependencies(
    known_ids: &HashSet<String>,
    deps_map: &HashMap<String, Vec<String>>,
    errors: &mut Vec<String>,
) {
    for (ticket_id, deps) in deps_map {
        for dep in deps {
            if !known_ids.contains(dep) {
                errors.push(format!(
                    ".waap/tickets/{ticket_id}/ticket.md depends_on {dep:?} which does not exist"
                ));
            }
        }
    }

    let mut visited: HashSet<String> = HashSet::new();
    let mut in_stack: HashSet<String> = HashSet::new();

    for ticket_id in known_ids {
        if !visited.contains(ticket_id) {
            let mut path = Vec::new();
            detect_cycle(
                ticket_id,
                deps_map,
                &mut visited,
                &mut in_stack,
                &mut path,
                errors,
            );
        }
    }
}

fn detect_cycle(
    id: &str,
    deps_map: &HashMap<String, Vec<String>>,
    visited: &mut HashSet<String>,
    in_stack: &mut HashSet<String>,
    path: &mut Vec<String>,
    errors: &mut Vec<String>,
) {
    visited.insert(id.to_string());
    in_stack.insert(id.to_string());
    path.push(id.to_string());

    if let Some(deps) = deps_map.get(id) {
        for dep in deps {
            if !visited.contains(dep.as_str()) {
                detect_cycle(dep, deps_map, visited, in_stack, path, errors);
            } else if in_stack.contains(dep.as_str()) {
                let cycle_start = path.iter().position(|p| p == dep).unwrap_or(0);
                let cycle_nodes: Vec<&str> =
                    path[cycle_start..].iter().map(|s| s.as_str()).collect();
                let cycle_str = format!("{} -> {}", cycle_nodes.join(" -> "), dep);
                errors.push(format!("dependency cycle detected: {cycle_str}"));
            }
        }
    }

    in_stack.remove(id);
    path.pop();
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
    fn missing_waap_state_fails_with_init_guidance() {
        let dir = tempdir().unwrap();

        assert_eq!(
            check_waap(dir.path()),
            vec!["no waap project found; run 'waap init'"]
        );
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
            "+++\ncreation_date = \"not a datetime\"\nstatus = \"ready\"\n+++\n",
        );

        let errors = check_waap(dir.path());

        assert!(errors
            .iter()
            .any(|error| error.contains("creation_date must be a TOML datetime")));
    }

    #[test]
    fn deprecated_role_field_is_tolerated() {
        let dir = tempdir().unwrap();
        fs::create_dir_all(dir.path().join(".waap/tickets")).unwrap();
        write_file(
            &dir.path().join(".waap/agents/aa-3881fda0/agent.md"),
            "+++\ncreation_date = 2026-06-18T15:00:34Z\nrole = \"designer\"\nstatus = \"ready\"\n+++\n\n# Purpose\n",
        );

        assert!(check_waap(dir.path()).is_empty());
    }

    #[test]
    fn unknown_ticket_field_fails_with_path_and_field() {
        let dir = tempdir().unwrap();
        write_file(
            &dir.path().join(".waap/tickets/tt-child/ticket.md"),
            "+++\ntitle = \"Child\"\ncreation_date = 2026-06-18T10:15:02Z\nstatus = \"pending\"\ndependencies = [\"tt-base\"]\n+++\n",
        );

        let errors = check_waap(dir.path());

        assert!(errors
            .iter()
            .any(|e| e.contains("unknown field dependencies")
                && e.contains(".waap/tickets/tt-child/ticket.md")));
    }

    #[test]
    fn unknown_agent_field_fails_with_path_and_field() {
        let dir = tempdir().unwrap();
        write_file(
            &dir.path().join(".waap/agents/aa-3881fda0/agent.md"),
            "+++\ncreation_date = 2026-06-18T15:00:34Z\nrole = \"developer\"\nstatus = \"ready\"\nworktree = \"some/path\"\n+++\n",
        );

        let errors = check_waap(dir.path());

        assert!(errors.iter().any(|e| e.contains("unknown field worktree")
            && e.contains(".waap/agents/aa-3881fda0/agent.md")));
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

    #[test]
    fn depends_on_missing_ticket_fails() {
        let dir = tempdir().unwrap();
        write_file(
            &dir.path().join(".waap/tickets/tt-child/ticket.md"),
            "+++\ntitle = \"Child\"\ncreation_date = 2026-06-18T10:15:02Z\nstatus = \"pending\"\ndepends_on = [\"tt-nonexistent\"]\n+++\n",
        );

        let errors = check_waap(dir.path());

        assert!(errors
            .iter()
            .any(|e| e.contains("tt-nonexistent") && e.contains("does not exist")));
    }

    #[test]
    fn depends_on_self_cycle_fails() {
        let dir = tempdir().unwrap();
        write_file(
            &dir.path().join(".waap/tickets/tt-self/ticket.md"),
            "+++\ntitle = \"Self\"\ncreation_date = 2026-06-18T10:15:02Z\nstatus = \"pending\"\ndepends_on = [\"tt-self\"]\n+++\n",
        );

        let errors = check_waap(dir.path());

        assert!(errors
            .iter()
            .any(|e| e.contains("cycle") && e.contains("tt-self")));
    }

    #[test]
    fn depends_on_two_ticket_cycle_fails() {
        let dir = tempdir().unwrap();
        write_file(
            &dir.path().join(".waap/tickets/tt-alpha/ticket.md"),
            "+++\ntitle = \"Alpha\"\ncreation_date = 2026-06-18T10:15:02Z\nstatus = \"pending\"\ndepends_on = [\"tt-beta\"]\n+++\n",
        );
        write_file(
            &dir.path().join(".waap/tickets/tt-beta/ticket.md"),
            "+++\ntitle = \"Beta\"\ncreation_date = 2026-06-18T10:15:02Z\nstatus = \"pending\"\ndepends_on = [\"tt-alpha\"]\n+++\n",
        );

        let errors = check_waap(dir.path());

        assert!(errors.iter().any(|e| e.contains("cycle")));
    }

    #[test]
    fn depends_on_valid_graph_passes() {
        let dir = tempdir().unwrap();
        write_file(
            &dir.path().join(".waap/tickets/tt-base/ticket.md"),
            "+++\ntitle = \"Base\"\ncreation_date = 2026-06-18T10:15:02Z\nstatus = \"completed\"\n+++\n",
        );
        write_file(
            &dir.path().join(".waap/tickets/tt-mid/ticket.md"),
            "+++\ntitle = \"Mid\"\ncreation_date = 2026-06-18T10:15:02Z\nstatus = \"pending\"\ndepends_on = [\"tt-base\"]\n+++\n",
        );
        write_file(
            &dir.path().join(".waap/tickets/tt-top/ticket.md"),
            "+++\ntitle = \"Top\"\ncreation_date = 2026-06-18T10:15:02Z\nstatus = \"pending\"\ndepends_on = [\"tt-base\", \"tt-mid\"]\n+++\n",
        );

        assert!(check_waap(dir.path()).is_empty());
    }

    #[test]
    fn depends_on_invalid_ticket_id_format_fails() {
        let dir = tempdir().unwrap();
        write_file(
            &dir.path().join(".waap/tickets/tt-child/ticket.md"),
            "+++\ntitle = \"Child\"\ncreation_date = 2026-06-18T10:15:02Z\nstatus = \"pending\"\ndepends_on = [\"not-a-ticket-id\"]\n+++\n",
        );

        let errors = check_waap(dir.path());

        assert!(errors
            .iter()
            .any(|e| e.contains("not-a-ticket-id") && e.contains("not a valid ticket id")));
    }

    fn write_file(path: &Path, contents: &str) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, contents).unwrap();
    }
}
