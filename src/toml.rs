use chrono::Utc;

use ::toml::Value;

pub(crate) fn current_toml_datetime() -> String {
    Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string()
}

pub(crate) fn datetime_string(value: &Value, key: &str) -> String {
    match value.get(key).expect("validated datetime") {
        Value::Datetime(datetime) => datetime.to_string(),
        _ => unreachable!("validated datetime"),
    }
}

pub(crate) fn toml_string(value: &str) -> String {
    use std::fmt::Write;

    // Physical newlines could expose a record's +++ delimiter inside a string.
    let mut escaped = String::from("\"");
    for character in value.chars() {
        match character {
            '\\' => escaped.push_str("\\\\"),
            '"' => escaped.push_str("\\\""),
            '\n' => escaped.push_str("\\n"),
            '\r' => escaped.push_str("\\r"),
            '\t' => escaped.push_str("\\t"),
            '\u{0000}'..='\u{001f}' | '\u{007f}' => {
                write!(escaped, "\\u{:04X}", character as u32).expect("write to string");
            }
            character => escaped.push(character),
        }
    }
    escaped.push('"');
    escaped
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serialized_strings_are_single_line_and_round_trip_all_ascii_characters() {
        let value: String = (0..=127).map(char::from).collect();
        let serialized = toml_string(&value);
        assert!(!serialized.contains(['\n', '\r']));
        let table: Value = format!("name = {serialized}").parse().unwrap();
        assert_eq!(table["name"].as_str(), Some(value.as_str()));
    }

    #[test]
    fn serialize_strings_round_trips_quotes_unicode_and_control_characters() {
        for value in ["plain", "café", "\"\\\n\r\t", "\0\x08\x0c\x1f\x7f"] {
            let table: Value = format!("name = {}", toml_string(value)).parse().unwrap();
            assert_eq!(table["name"].as_str(), Some(value));
        }
    }
}
