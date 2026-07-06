use std::io;
use std::path::Path;

use chrono::Utc;

pub(crate) fn random_hex_chars(hex_char_count: usize) -> io::Result<String> {
    let mut bytes = vec![0_u8; hex_char_count / 2];
    getrandom::getrandom(&mut bytes).map_err(|error| io::Error::other(error.to_string()))?;
    Ok(bytes.iter().map(|byte| format!("{byte:02x}")).collect())
}

pub(crate) fn available_record_id(
    records_dir: &Path,
    prefix: &str,
    name: Option<&str>,
) -> io::Result<String> {
    if let Some(name) = name {
        let slug = slugify_name(name)?;
        let record_id = format!("{prefix}{slug}");
        if !records_dir.join(&record_id).exists() {
            return Ok(record_id);
        }

        loop {
            let record_id = format!(
                "{prefix}{}",
                slug_with_hash_from_slug(&slug, &random_hex_chars(4)?)
            );
            if !records_dir.join(&record_id).exists() {
                return Ok(record_id);
            }
        }
    }

    loop {
        let record_id = format!("{prefix}{}", random_hex_chars(8)?);
        if !records_dir.join(&record_id).exists() {
            return Ok(record_id);
        }
    }
}

pub(crate) fn slugify_name(name: &str) -> io::Result<String> {
    let mut slug = String::new();
    let mut previous_dash = false;

    for byte in name.trim().bytes() {
        match byte {
            b'A'..=b'Z' => {
                slug.push((byte + 32) as char);
                previous_dash = false;
            }
            b'a'..=b'z' | b'0'..=b'9' => {
                slug.push(byte as char);
                previous_dash = false;
            }
            b' ' | b'\t' | b'\n' | b'\r' | b'-' if !slug.is_empty() && !previous_dash => {
                slug.push('-');
                previous_dash = true;
            }
            _ => {}
        }
    }

    while slug.ends_with('-') {
        slug.pop();
    }

    if slug.is_empty() {
        slug.push_str("ticket");
    }

    if slug.len() > 63 {
        slug = slug_with_hash_from_slug(&slug, &random_hex_chars(4)?);
    }

    Ok(slug)
}

fn slug_with_hash_from_slug(slug: &str, hash: &str) -> String {
    let prefix_len = slug.len().min(58);
    let mut prefix = slug[..prefix_len].trim_end_matches('-').to_string();
    if prefix.is_empty() {
        prefix.push_str("ticket");
    }
    format!("{prefix}-{hash}")
}

pub(crate) fn is_record_id(value: &str, prefix: &str) -> bool {
    let Some(slug) = value.strip_prefix(prefix) else {
        return false;
    };

    !slug.is_empty()
        && slug.len() < 64
        && !slug.starts_with('-')
        && !slug.ends_with('-')
        && !slug.contains("--")
        && slug
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
}

pub(crate) fn current_toml_datetime() -> String {
    Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string()
}

pub(crate) fn toml_string(value: &str) -> String {
    let mut escaped = String::from("\"");
    for character in value.chars() {
        match character {
            '\\' => escaped.push_str("\\\\"),
            '"' => escaped.push_str("\\\""),
            '\n' => escaped.push_str("\\n"),
            '\r' => escaped.push_str("\\r"),
            '\t' => escaped.push_str("\\t"),
            character => escaped.push(character),
        }
    }
    escaped.push('"');
    escaped
}

#[cfg(test)]
mod tests {
    use std::fs;

    use tempfile::tempdir;

    use super::{available_record_id, is_record_id, slugify_name};

    #[test]
    fn slug_generation_matches_shared_rules() {
        assert_eq!(
            slugify_name("  List All Tickets!  ").unwrap(),
            "list-all-tickets"
        );
        assert_eq!(
            slugify_name("Bad---Spaces   Here").unwrap(),
            "bad-spaces-here"
        );
        assert_eq!(slugify_name("Café: déjà_vu").unwrap(), "caf-djvu");
    }

    #[test]
    fn long_slug_is_truncated_with_hex_hash() {
        let slug = slugify_name(
            "This is a very long record name that should be truncated because it exceeds limits",
        )
        .unwrap();

        assert!(slug.len() <= 63);
        assert_eq!(slug.as_bytes()[slug.len() - 5], b'-');
        assert!(slug[slug.len() - 4..]
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase()));
    }

    #[test]
    fn named_conflict_appends_hex_hash() {
        let dir = tempdir().unwrap();
        fs::create_dir_all(dir.path().join("tt-list-tickets")).unwrap();

        let record_id = available_record_id(dir.path(), "tt-", Some("List Tickets")).unwrap();

        assert!(record_id.starts_with("tt-list-tickets-"));
        assert!(is_record_id(&record_id, "tt-"));
    }

    #[test]
    fn unnamed_record_uses_eight_hex_characters() {
        let dir = tempdir().unwrap();

        let record_id = available_record_id(dir.path(), "aa-", None).unwrap();
        let suffix = record_id.strip_prefix("aa-").unwrap();

        assert_eq!(suffix.len(), 8);
        assert!(suffix
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase()));
    }
}
