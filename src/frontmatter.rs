use std::fs::File;
use std::io::{self, BufRead, BufReader};
use std::path::Path;

use ::toml::Value;

pub(crate) fn serialize_record(frontmatter_lines: &str, body: &str) -> String {
    format!("+++\n{frontmatter_lines}+++\n{body}")
}

pub(crate) fn invalid_frontmatter_error(errors: Vec<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, errors.join("; "))
}

pub(crate) fn parse_frontmatter(path: &Path, errors: &mut Vec<String>) -> Option<Value> {
    let file = match File::open(path) {
        Ok(file) => file,
        Err(error) => {
            errors.push(format!("failed to read {}: {error}", path.display()));
            return None;
        }
    };
    let mut lines = BufReader::new(file).lines();
    match lines.next() {
        Some(Ok(line)) if line == "+++" => {}
        Some(Ok(_)) | None => {
            errors.push(format!(
                "{} must start with TOML frontmatter delimited by +++",
                path.display()
            ));
            return None;
        }
        Some(Err(error)) => {
            errors.push(format!("failed to read {}: {error}", path.display()));
            return None;
        }
    }

    let mut frontmatter = String::new();
    for line in lines {
        match line {
            Ok(line) if line == "+++" => {
                return parse_frontmatter_toml(&frontmatter, path, errors);
            }
            Ok(line) => {
                frontmatter.push_str(&line);
                frontmatter.push('\n');
            }
            Err(error) => {
                errors.push(format!("failed to read {}: {error}", path.display()));
                return None;
            }
        }
    }

    errors.push(format!(
        "{} frontmatter is missing closing +++ delimiter",
        path.display()
    ));
    None
}

#[allow(dead_code)]
pub(crate) fn parse_frontmatter_from_contents(
    contents: &str,
    path: &Path,
    errors: &mut Vec<String>,
) -> Option<Value> {
    let mut lines = contents.lines();
    if lines.next() != Some("+++") {
        errors.push(format!(
            "{} must start with TOML frontmatter delimited by +++",
            path.display()
        ));
        return None;
    }

    let mut frontmatter = String::new();
    for line in lines {
        if line == "+++" {
            return parse_frontmatter_toml(&frontmatter, path, errors);
        }
        frontmatter.push_str(line);
        frontmatter.push('\n');
    }

    errors.push(format!(
        "{} frontmatter is missing closing +++ delimiter",
        path.display()
    ));
    None
}

fn parse_frontmatter_toml(
    frontmatter: &str,
    path: &Path,
    errors: &mut Vec<String>,
) -> Option<Value> {
    match frontmatter.parse::<Value>() {
        Ok(value) if value.is_table() => Some(value),
        Ok(_) => {
            errors.push(format!(
                "{} frontmatter must be a TOML table",
                path.display()
            ));
            None
        }
        Err(error) => {
            errors.push(format!(
                "{} frontmatter is invalid TOML: {error}",
                path.display()
            ));
            None
        }
    }
}

pub(crate) fn reject_unknown_fields(
    frontmatter: &Value,
    allowed: &[&str],
    path: &Path,
    errors: &mut Vec<String>,
) {
    let Some(table) = frontmatter.as_table() else {
        return;
    };
    for key in table.keys() {
        if !allowed.contains(&key.as_str()) {
            errors.push(format!(
                "{} frontmatter has unknown field {key}; allowed fields are {}",
                path.display(),
                allowed.join(", ")
            ));
        }
    }
}

pub(crate) fn require_optional_string(
    frontmatter: &Value,
    key: &str,
    path: &Path,
    errors: &mut Vec<String>,
) {
    if frontmatter
        .get(key)
        .is_some_and(|value| value.as_str().is_none())
    {
        errors.push(format!(
            "{} frontmatter field {key} must be a string",
            path.display()
        ));
    }
}

pub(crate) fn require_optional_string_choice(
    frontmatter: &Value,
    key: &str,
    choices: &[&str],
    path: &Path,
    errors: &mut Vec<String>,
) {
    match frontmatter.get(key).map(Value::as_str) {
        None => {}
        Some(Some(value)) if choices.contains(&value) => {}
        Some(Some(value)) => errors.push(format!(
            "{} frontmatter field {key} has invalid value {value:?}; expected one of {}",
            path.display(),
            choices.join(", ")
        )),
        Some(None) => errors.push(format!(
            "{} frontmatter field {key} must be a string",
            path.display()
        )),
    }
}

pub(crate) fn require_string_choice(
    frontmatter: &Value,
    key: &str,
    choices: &[&str],
    path: &Path,
    errors: &mut Vec<String>,
) {
    match frontmatter.get(key).and_then(Value::as_str) {
        Some(value) if choices.contains(&value) => {}
        Some(value) => errors.push(format!(
            "{} frontmatter field {key} has invalid value {value:?}; expected one of {}",
            path.display(),
            choices.join(", ")
        )),
        None => errors.push(format!(
            "{} frontmatter field {key} must be a string",
            path.display()
        )),
    }
}

pub(crate) fn require_optional_string_array(
    frontmatter: &Value,
    key: &str,
    path: &Path,
    errors: &mut Vec<String>,
) {
    match frontmatter.get(key) {
        None => {}
        Some(Value::Array(arr)) => {
            for (i, element) in arr.iter().enumerate() {
                if element.as_str().is_none() {
                    errors.push(format!(
                        "{} frontmatter field {key}[{i}] must be a string",
                        path.display()
                    ));
                }
            }
        }
        Some(_) => errors.push(format!(
            "{} frontmatter field {key} must be an array of strings",
            path.display()
        )),
    }
}

pub(crate) fn require_datetime(
    frontmatter: &Value,
    key: &str,
    path: &Path,
    errors: &mut Vec<String>,
) {
    if !frontmatter
        .get(key)
        .is_some_and(|value| matches!(value, Value::Datetime(_)))
    {
        errors.push(format!(
            "{} frontmatter field {key} must be a TOML datetime",
            path.display()
        ));
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use tempfile::tempdir;

    use super::parse_frontmatter;

    #[test]
    fn file_parser_stops_after_closing_delimiter() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("record.md");
        let mut contents = b"+++\nstatus = \"ready\"\n+++\n".to_vec();
        contents.push(0xff);
        fs::write(&path, contents).unwrap();

        let mut errors = Vec::new();
        let frontmatter = parse_frontmatter(&path, &mut errors).unwrap();

        assert!(errors.is_empty());
        assert_eq!(frontmatter["status"].as_str(), Some("ready"));
    }

    #[test]
    fn file_parser_reports_missing_opening_delimiter() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("record.md");
        fs::write(&path, "status = \"ready\"\n").unwrap();

        let mut errors = Vec::new();
        assert!(parse_frontmatter(&path, &mut errors).is_none());

        assert_eq!(
            errors,
            vec![format!(
                "{} must start with TOML frontmatter delimited by +++",
                path.display()
            )]
        );
    }

    #[test]
    fn file_parser_reports_missing_closing_delimiter() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("record.md");
        fs::write(&path, "+++\nstatus = \"ready\"\n").unwrap();

        let mut errors = Vec::new();
        assert!(parse_frontmatter(&path, &mut errors).is_none());

        assert_eq!(
            errors,
            vec![format!(
                "{} frontmatter is missing closing +++ delimiter",
                path.display()
            )]
        );
    }
}
