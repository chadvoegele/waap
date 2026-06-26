use std::fs;
use std::io;
use std::path::Path;

use toml::Value;

pub(crate) fn serialize_record(frontmatter_lines: &str, body: &str) -> String {
    format!("+++\n{frontmatter_lines}+++\n{body}")
}

pub(crate) fn invalid_frontmatter_error(errors: Vec<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, errors.join("; "))
}

pub(crate) fn datetime_string(frontmatter: &Value, key: &str) -> String {
    match frontmatter.get(key).expect("validated datetime") {
        Value::Datetime(datetime) => datetime.to_string(),
        _ => unreachable!("validated datetime"),
    }
}

pub(crate) fn parse_frontmatter(path: &Path, errors: &mut Vec<String>) -> Option<Value> {
    let contents = match fs::read_to_string(path) {
        Ok(contents) => contents,
        Err(error) => {
            errors.push(format!("failed to read {}: {error}", path.display()));
            return None;
        }
    };
    parse_frontmatter_from_contents(&contents, path, errors)
}

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
            return match frontmatter.parse::<Value>() {
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
            };
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

pub(crate) fn require_string(
    frontmatter: &Value,
    key: &str,
    path: &Path,
    errors: &mut Vec<String>,
) {
    if frontmatter.get(key).and_then(Value::as_str).is_none() {
        errors.push(format!(
            "{} frontmatter field {key} must be a string",
            path.display()
        ));
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
