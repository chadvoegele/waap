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
