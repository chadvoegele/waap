use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use crate::agent::is_agent_id;
use crate::ticket::is_ticket_id;

#[derive(Clone, Copy, Debug)]
pub(crate) enum WaapRecordKind {
    Agent,
    Ticket,
}

impl WaapRecordKind {
    pub(crate) fn directory_name(self) -> &'static str {
        match self {
            WaapRecordKind::Agent => "agents",
            WaapRecordKind::Ticket => "tickets",
        }
    }

    pub(crate) fn singular_name(self) -> &'static str {
        match self {
            WaapRecordKind::Agent => "agent",
            WaapRecordKind::Ticket => "ticket",
        }
    }

    pub(crate) fn directory_description(self) -> &'static str {
        match self {
            WaapRecordKind::Agent => "an agent directory",
            WaapRecordKind::Ticket => "a ticket directory",
        }
    }

    pub(crate) fn id_example(self) -> &'static str {
        match self {
            WaapRecordKind::Agent => "aa-3881fda0",
            WaapRecordKind::Ticket => "tt-list-tickets",
        }
    }

    pub(crate) fn is_valid_id(self, value: &str) -> bool {
        match self {
            WaapRecordKind::Agent => is_agent_id(value),
            WaapRecordKind::Ticket => is_ticket_id(value),
        }
    }

    pub(crate) fn root_label(self) -> String {
        format!(".waap/{}", self.directory_name())
    }

    pub(crate) fn root_path(self, repo_root: &Path) -> PathBuf {
        repo_root.join(".waap").join(self.directory_name())
    }
}

pub(crate) fn list_record_ids(repo_root: &Path, kind: WaapRecordKind) -> io::Result<Vec<String>> {
    let records_dir = kind.root_path(repo_root);
    let records_label = kind.root_label();
    if !records_dir.exists() {
        return Ok(Vec::new());
    }
    if !records_dir.is_dir() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("{records_label} must be a directory"),
        ));
    }

    let mut ids = Vec::new();
    for entry in fs::read_dir(&records_dir)? {
        let entry = entry?;
        let path = entry.path();
        let id = entry.file_name().to_string_lossy().into_owned();
        let label = format!("{records_label}/{id}");

        if !path.is_dir() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("{label} must be {}", kind.directory_description()),
            ));
        }
        if !kind.is_valid_id(&id) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!(
                    "{label} must be named as a {} id like {}",
                    kind.singular_name(),
                    kind.id_example()
                ),
            ));
        }

        ids.push(id);
    }
    ids.sort();

    Ok(ids)
}

const NESTED_SCAN_SKIP_DIRS: &[&str] = &[".git", "worktrees", "target", "node_modules"];

/// Find nested `.waap` directories below `repo_root`, excluding `repo_root/.waap` itself.
///
/// Does not descend into a `.waap` directory once found, and skips `.git`, `worktrees`,
/// `target`, and `node_modules` anywhere in the tree.
pub(crate) fn find_nested_waap_projects(repo_root: &Path) -> Vec<PathBuf> {
    let mut nested = Vec::new();
    find_nested_waap_projects_in_dir(repo_root, repo_root, &mut nested);
    nested
}

fn find_nested_waap_projects_in_dir(repo_root: &Path, dir: &Path, nested: &mut Vec<PathBuf>) {
    let entries = match fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(_) => return,
    };

    for entry in entries {
        let entry = match entry {
            Ok(entry) => entry,
            Err(_) => continue,
        };
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }

        let name = entry.file_name();
        let name = name.to_string_lossy();

        if NESTED_SCAN_SKIP_DIRS.contains(&name.as_ref()) {
            continue;
        }

        if name == ".waap" {
            if path != repo_root.join(".waap") {
                nested.push(path);
            }
            continue;
        }

        find_nested_waap_projects_in_dir(repo_root, &path, nested);
    }
}

pub(crate) fn markdown_body_after_frontmatter(contents: &str) -> io::Result<String> {
    let mut delimiter_count = 0;
    let mut offset = 0;

    for line in contents.split_inclusive('\n') {
        let line_without_newline = line.trim_end_matches('\n').trim_end_matches('\r');
        if line_without_newline == "+++" {
            delimiter_count += 1;
            if delimiter_count == 2 {
                return Ok(contents[offset + line.len()..].to_string());
            }
        }
        offset += line.len();
    }

    if contents[offset..].trim_end_matches('\r') == "+++" {
        return Ok(String::new());
    }

    Err(io::Error::new(
        io::ErrorKind::InvalidData,
        "frontmatter is missing closing +++ delimiter",
    ))
}

#[cfg(test)]
mod tests {
    use std::fs;

    use tempfile::tempdir;

    use super::find_nested_waap_projects;

    #[test]
    fn finds_nested_waap_project() {
        let dir = tempdir().unwrap();
        fs::create_dir_all(dir.path().join(".waap")).unwrap();
        fs::create_dir_all(dir.path().join("wiki/.waap")).unwrap();

        let nested = find_nested_waap_projects(dir.path());

        assert_eq!(nested, vec![dir.path().join("wiki/.waap")]);
    }

    #[test]
    fn finds_no_nested_waap_project() {
        let dir = tempdir().unwrap();
        fs::create_dir_all(dir.path().join(".waap")).unwrap();
        fs::create_dir_all(dir.path().join("wiki")).unwrap();

        let nested = find_nested_waap_projects(dir.path());

        assert!(nested.is_empty());
    }

    #[test]
    fn excludes_repo_root_waap_directory() {
        let dir = tempdir().unwrap();
        fs::create_dir_all(dir.path().join(".waap/tickets")).unwrap();

        let nested = find_nested_waap_projects(dir.path());

        assert!(nested.is_empty());
    }

    #[test]
    fn skips_bounded_directories() {
        let dir = tempdir().unwrap();
        fs::create_dir_all(dir.path().join(".waap")).unwrap();
        fs::create_dir_all(dir.path().join(".git/.waap")).unwrap();
        fs::create_dir_all(dir.path().join("worktrees/.waap")).unwrap();
        fs::create_dir_all(dir.path().join("target/.waap")).unwrap();
        fs::create_dir_all(dir.path().join("node_modules/.waap")).unwrap();

        let nested = find_nested_waap_projects(dir.path());

        assert!(nested.is_empty());
    }

    #[test]
    fn does_not_descend_into_a_found_waap_directory() {
        let dir = tempdir().unwrap();
        fs::create_dir_all(dir.path().join(".waap")).unwrap();
        fs::create_dir_all(dir.path().join("wiki/.waap/tickets/.waap")).unwrap();

        let nested = find_nested_waap_projects(dir.path());

        assert_eq!(nested, vec![dir.path().join("wiki/.waap")]);
    }
}
