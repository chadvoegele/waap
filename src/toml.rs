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
    Value::String(value.to_string()).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serialize_strings_round_trips_quotes_unicode_and_control_characters() {
        for value in ["plain", "café", "\"\\\n\r\t", "\0\x08\x0c\x1f\x7f"] {
            let table: Value = format!("name = {}", toml_string(value)).parse().unwrap();
            assert_eq!(table["name"].as_str(), Some(value));
        }
    }
}
